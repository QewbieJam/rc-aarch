/*
 * ivshm-console —— rc-aarch ch7+ phase 1 的 root 侧测试程序
 *
 * 通过 uio_ivshmem 的 /dev/uio0 访问与 cell3 共享的 ivshmem 内存，
 * 用和 guest（os-cell3/src/ivshmem.rs）相同的简单字节环协议收发。
 *
 * 用法（在 root cell 上跑）：
 *   ./ivshm-console -s "ping from root" -d 5        # 发一句，收 5 秒
 *   ./ivshm-console -d 10 > /tmp/ivshm.log &         # 只收
 *
 * 共享段（Jailhouse ivshmem-net 的 peer 0 = root 视角）：
 *   uio map2 = input_sections @ 0x6ffff000，2 段只读：
 *        +0x00000  peer0（root 自己）
 *        +0x7f000  peer1（guest 的 TX）← 我们从这里读
 *   uio map3 = output_section @ 0x6ffff000，可写：
 *        root 自己的 TX（写进去 guest 读）→ 我们往这里写
 *
 * 段内布局（见 os-cell3/src/ivshmem.rs）：
 *   off 0   magic: u32  'RCSH'
 *   off 4   write: u32  生产者下标
 *   off 8   read:  u32  消费者下标
 *   off 12  cap:   u32
 *   off 16  data[cap]
 */
#include <errno.h>
#include <fcntl.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <poll.h>
#include <time.h>
#include <unistd.h>

#define MAGIC 0x52435348u /* 'R''C''S''H' */
#define SECTION 0x7f000u
#define HDR 20u
#define CAP (SECTION - HDR)

#define OFF_MAGIC 0u
#define OFF_GEN 4u
#define OFF_WRITE 8u
#define OFF_READ 12u
#define OFF_CAP 16u

static volatile uint8_t *root_tx;  /* 我们写、guest 读（map3） */
static volatile uint8_t *guest_tx; /* guest 写、我们读（map2 + 0x7f000） */

static inline uint32_t rd32(volatile uint8_t *p, unsigned off)
{
	return __atomic_load_n((uint32_t *)(p + off), __ATOMIC_RELAXED);
}

static inline void wr32(volatile uint8_t *p, unsigned off, uint32_t v)
{
	__atomic_store_n((uint32_t *)(p + off), v, __ATOMIC_RELAXED);
}

static inline uint32_t ld_acq(volatile uint8_t *p, unsigned off)
{
	return __atomic_load_n((uint32_t *)(p + off), __ATOMIC_ACQUIRE);
}

static inline void st_rel(volatile uint8_t *p, unsigned off, uint32_t v)
{
	__atomic_store_n((uint32_t *)(p + off), v, __ATOMIC_RELEASE);
}

static uint32_t last_guest_gen;

static void ring_init(volatile uint8_t *s)
{
	struct timespec ts;

	clock_gettime(CLOCK_MONOTONIC, &ts);
	wr32(s, OFF_CAP, CAP);
	wr32(s, OFF_WRITE, 0);
	wr32(s, OFF_READ, 0);
	wr32(s, OFF_GEN, (uint32_t)(ts.tv_nsec ^ (ts.tv_sec << 20)));
	__atomic_thread_fence(__ATOMIC_SEQ_CST);
	wr32(s, OFF_MAGIC, MAGIC);
	__atomic_thread_fence(__ATOMIC_SEQ_CST);
}

/*
 * 下标归属（和 guest 侧 ivshmem.rs 一致）：
 *   每个 cell 只能写自己的段。所以"我读到哪儿"的 read 下标写在自己的段里，
 *   而 free/avail 要去看**对方段里的 read**（对方消费到哪儿）。
 *   own = 自己的输出段，peer = 对方的输出段。
 */
static size_t tx_write(volatile uint8_t *own, volatile uint8_t *peer,
		       const uint8_t *buf, size_t len)
{
	uint32_t w = rd32(own, OFF_WRITE);
	uint32_t r = ld_acq(peer, OFF_READ);
	uint32_t used = w - r;

	if (used > CAP) /* 对方还没看到我们的新 gen，read 是上一代的 */
		used = 0;
	if (used >= CAP)
		return 0;
	if (len > CAP - used)
		len = CAP - used;
	for (size_t i = 0; i < len; i++)
		own[HDR + (w + i) % CAP] = buf[i];
	st_rel(own, OFF_WRITE, w + (uint32_t)len);
	return len;
}

static size_t rx_read(volatile uint8_t *peer, volatile uint8_t *own,
		      uint8_t *buf, size_t len)
{
	uint32_t r, w, avail;

	if (rd32(peer, OFF_MAGIC) != MAGIC)
		return 0;
	if (rd32(peer, OFF_GEN) != last_guest_gen) {
		last_guest_gen = rd32(peer, OFF_GEN);
		wr32(own, OFF_READ, 0);
	}
	r = rd32(own, OFF_READ);
	w = ld_acq(peer, OFF_WRITE);
	avail = w - r;
	if (avail == 0 || avail > CAP)
		return 0;
	if (len > avail)
		len = avail;
	for (size_t i = 0; i < len; i++)
		buf[i] = peer[HDR + (r + i) % CAP];
	st_rel(own, OFF_READ, r + (uint32_t)len);
	return len;
}

static void *map_region(int fd, unsigned index, size_t size, int prot)
{
	void *p = mmap(NULL, size, prot, MAP_SHARED, fd, (off_t)index * 4096);

	if (p == MAP_FAILED) {
		fprintf(stderr, "mmap(map%u) 失败: %s\n", index, strerror(errno));
		exit(1);
	}
	return p;
}

static size_t sysfs_size(unsigned index)
{
	char path[128];
	char buf[64];
	size_t n;
	int fd;

	snprintf(path, sizeof(path),
		 "/sys/class/uio/uio0/maps/map%u/size", index);
	fd = open(path, O_RDONLY);
	if (fd < 0)
		return 0;
	n = read(fd, buf, sizeof(buf) - 1);
	close(fd);
	if (n <= 0)
		return 0;
	buf[n] = 0;
	return strtoull(buf, NULL, 0);
}

static void die(const char *msg)
{
	fprintf(stderr, "%s: %s\n", msg, strerror(errno));
	exit(1);
}

static double now_sec(void)
{
	struct timespec ts;

	clock_gettime(CLOCK_MONOTONIC, &ts);
	return ts.tv_sec + ts.tv_nsec / 1e9;
}

int main(int argc, char **argv)
{
	const char *send_text = NULL;
	double duration = 3.0;
	int rounds = 1;
	int interactive = 0;
	int reset_only = 0;
	size_t in_sz, out_sz;
	uint8_t buf[4096];
	double deadline;
	int fd;

	for (int i = 1; i < argc; i++) {
		if (strcmp(argv[i], "-s") == 0 && i + 1 < argc)
			send_text = argv[++i];
		else if (strcmp(argv[i], "-d") == 0 && i + 1 < argc)
			duration = atof(argv[++i]);
		else if (strcmp(argv[i], "-n") == 0 && i + 1 < argc)
			rounds = atoi(argv[++i]);
		else if (strcmp(argv[i], "-i") == 0)
			interactive = 1;
		else if (strcmp(argv[i], "-r") == 0)
			reset_only = 1;
		else {
			fprintf(stderr, "用法: %s [-s 文本] [-d 秒]\n", argv[0]);
			return 2;
		}
	}

	fd = open("/dev/uio0", O_RDWR);
	if (fd < 0)
		die("打开 /dev/uio0");
	in_sz = sysfs_size(2);
	out_sz = sysfs_size(3);
	if (in_sz < 2 * SECTION || out_sz < SECTION) {
		fprintf(stderr, "uio0 的 map 尺寸不对: map2=%zu map3=%zu\n",
			in_sz, out_sz);
		return 1;
	}

	root_tx = map_region(fd, 3, out_sz, PROT_READ | PROT_WRITE);
	guest_tx = (volatile uint8_t *)map_region(fd, 2, in_sz, PROT_READ) +
		   0x7f000;

	ring_init(root_tx);
	if (reset_only) {
		fprintf(stderr, "[ivshm] 已重置 root 段（丢弃上一轮残留输入）\n");
		close(fd);
		return 0;
	}
	/* 启动前 ring 里可能还有上一次运行的残留：把 read 推到 write，先丢弃 */
	if (rd32(guest_tx, OFF_MAGIC) == MAGIC) {
		last_guest_gen = rd32(guest_tx, OFF_GEN);
		wr32(root_tx, OFF_READ, rd32(guest_tx, OFF_WRITE));
	}
	/* 等 guest 把它的段初始化好（magic 出现）再发；gen 变了说明它这次真的启动了 */
	{
		uint32_t gen0 = rd32(guest_tx, OFF_GEN);
		double t0 = now_sec();

		while (now_sec() - t0 < 5.0) {
			if (rd32(guest_tx, OFF_MAGIC) == MAGIC &&
			    rd32(guest_tx, OFF_GEN) != gen0)
				break;
			usleep(2000);
		}
		if (rd32(guest_tx, OFF_GEN) == gen0)
			fprintf(stderr, "[ivshm] 5s 内没等到 guest 新 gen，照样发\n");
	}
	fprintf(stderr, "[ivshm] root TX @ %p, guest TX @ %p\n",
		(void *)root_tx, (void *)guest_tx);

	/*
	 * 等 guest 把它的段初始化好（magic 出现 + gen 变化）再发：
	 *   - magic 可能是上一次运行留下的（共享内存不清零），不能只看 magic；
	 *   - gen 变了才说明 guest 这次真的启动了；
	 *   - 如果 5s 内 gen 没变，可能 guest 已经先启动过了，照发。
	 */


	if (send_text && rounds > 1) {
		/*
		 * round-trip 自检：发 "ping-<i>\n"，等 guest 回显同样的内容。
		 * 收到的数据可能被合并（一次 read 里带好几行）或者夹杂 guest 的
		 * 启动问候，所以按"行"累积、逐行匹配，非期望的行丢弃。
		 */
		int ok = 0;
		uint8_t acc[8192];
		size_t acc_len = 0;

		for (int i = 0; i < rounds; i++) {
			char line[64];
			int len = snprintf(line, sizeof(line), "ping-%03d\n", i);
			double t0;

			while (tx_write(root_tx, guest_tx, (const uint8_t *)line,
					len) == 0)
				usleep(200);
			t0 = now_sec();
			for (;;) {
				uint8_t got[512];
				size_t n;
				int matched = 0;

				n = rx_read(guest_tx, root_tx, got, sizeof(got));
				if (n > 0 && acc_len + n < sizeof(acc)) {
					memcpy(acc + acc_len, got, n);
					acc_len += n;
				}
				/* 先看累积缓冲里有没有期望的行（按行处理，非期望的行丢弃） */
				while (acc_len > 0) {
					uint8_t *nl = memchr(acc, '\n', acc_len);
					size_t linelen;

					if (!nl)
						break; /* 半行，等更多数据 */
					linelen = (size_t)(nl - acc) + 1;
					if (linelen == (size_t)len &&
					    memcmp(acc, line, len) == 0) {
						matched = 1;
					} else {
						fprintf(stderr, "[ivshm] 丢弃: %.*s",
							(int)linelen, acc);
					}
					memmove(acc, acc + linelen, acc_len - linelen);
					acc_len -= linelen;
					if (matched)
						break;
				}
				if (matched) {
					ok++;
					break;
				}
				if (now_sec() - t0 > 2.0) {
					fprintf(stderr,
						"[ivshm] 第 %d 次超时（没收到回显）\n",
						i);
					break;
				}
				usleep(500);
			}
		}
		fprintf(stderr, "[ivshm] round-trip %d/%d 成功\n", ok, rounds);
	} else if (send_text) {
		size_t n = tx_write(root_tx, guest_tx, (const uint8_t *)send_text,
				    strlen(send_text));
		fprintf(stderr, "[ivshm] 已发送 %zu 字节: %s\n", n, send_text);
	}

	deadline = now_sec() + duration;
	if (interactive) {
		/* phase 2：把本进程的 stdin 转发进 guest，把 guest 的输出打到 stdout */
		int stdin_open = 1;

		fprintf(stderr, "[ivshm] 交互模式：stdin → guest，guest 输出 → stdout\n");
		while (now_sec() < deadline) {
			struct pollfd pfd = {.fd = 0, .events = POLLIN};
			uint8_t ibuf[512];
			int pr = poll(&pfd, 1, 10);
			size_t n;

			if (stdin_open && pr > 0 &&
			    (pfd.revents & (POLLIN | POLLHUP | POLLERR))) {
				ssize_t r = read(0, ibuf, sizeof(ibuf));

				if (r > 0) {
					size_t w = tx_write(root_tx, guest_tx, ibuf,
							    (size_t)r);
					fprintf(stderr,
						"[ivshm] stdin→guest 转发 %zu/%zd 字节\n",
						w, r);
				}
				else if (r == 0)
					stdin_open = 0;
			}
			n = rx_read(guest_tx, root_tx, ibuf, sizeof(ibuf));
			if (n > 0) {
				fwrite(ibuf, 1, n, stdout);
				fflush(stdout);
			}
		}
		close(fd);
		return 0;
	}
	while (now_sec() < deadline) {
		size_t n = rx_read(guest_tx, root_tx, buf, sizeof(buf));

		if (n > 0) {
			fwrite(buf, 1, n, stdout);
			fflush(stdout);
		} else {
			usleep(1000);
		}
	}
	close(fd);
	return 0;
}
