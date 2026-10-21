/*
 * rc-aarch cell3: Rust 裸机 inmate（Raspberry Pi 4 / openEuler MCS）
 *
 * 参考上游 jailhouse configs/arm64/rpi4-inmate-demo.c，按我们的环境改：
 *   - CPU3（0b1000）：root cell 用 0-1，guest 用 2-3 → ch1 阶段先不起 guest 就不会冲突
 *   - 加载/运行区 0xFA000000 + 32MB：落在 root cell 声明的 0x80000000-0xFC000000 内，
 *     但 root Linux 有 mem=750M，永远不会碰这里 → 不用改任何现有 cell 的内存布局
 *   - 控制台：8250@0xFE215040（mini UART，正是 hypervisor 的 debug_console 同一设备，
 *     也是 GPIO14/15 上你接的那条串口）→ 输出你能直接在串口看到
 *   - 开 VIRTUAL_CONSOLE_ACTIVE：允许用 hvc #0x4a48 打印，输出还能在 root cell 里
 *     用 `cat /sys/devices/jailhouse/console` 回看
 *
 * 注意：所有地址都是"cell 视角"的物理地址（ch1 不开 MMU），必须和内核 linker.ld 一致。
 *
 * ✅ 2026-09-23 实测：jailhouse enable 后 `jailhouse cell create` 成功
 *    （开机日志：Initializing processors CPU3 OK … Created cell "rc-aarch"），
 *    root cell 变为 0-2、rc-aarch 拿到 CPU3。
 *    此前多次 Invalid argument 的真因是【jailhouse 未使能】，配置本身无误。
 */
#include <jailhouse/types.h>
#include <jailhouse/cell-config.h>

struct {
	struct jailhouse_cell_desc cell;
	__u64 cpus[1];
	struct jailhouse_memory mem_regions[8];
	struct jailhouse_irqchip irqchips[0];
} __attribute__((packed)) config = {
	.cell = {
		.signature = JAILHOUSE_CELL_DESC_SIGNATURE,
		.revision = JAILHOUSE_CONFIG_REVISION,
		.name = "rc-aarch",
		.flags = JAILHOUSE_CELL_PASSIVE_COMMREG |
			 JAILHOUSE_CELL_VIRTUAL_CONSOLE_ACTIVE,

		.cpu_set_size = sizeof(config.cpus),
		.num_memory_regions = ARRAY_SIZE(config.mem_regions),
		.num_irqchips = ARRAY_SIZE(config.irqchips),

		/*
		 * ★ 裸核 cell 必须显式给出"上电入口地址"：hypervisor 在 arch_cell_reset()
		 *   里执行 public_per_cpu(first)->cpu_on_entry = config->cpu_reset_address。
		 *   不写就默认 0 → start 时 CPU 跳到 0x0 → "instruction abort at 0x0"。
		 *   这里必须 = 内核 linker.ld 的基址 = LOADABLE 区的 virt_start。
		 *   （对照：configs/arm64/rpi4-zephyr.c 里是 0x7a000000）
		 */
		.cpu_reset_address = 0xfa000000,

		/* ch1 不需要中断，ch3 再回来加 irqchips + GIC */
		.console = {
			.address = 0xfe215040,
			.type = JAILHOUSE_CON_TYPE_8250,
			.flags = JAILHOUSE_CON_ACCESS_MMIO |
				 JAILHOUSE_CON_REGDIST_4,
		},
	},

	.cpus = {
		0b1000,			/* CPU3 */
	},

	.mem_regions = {
		/*
		 * ch7+ ivshmem：和 root cell 共享的 4 段内存（peer 1 视角）
		 *   [0] 0x6fffe000 0x1000  状态表（hypervisor 维护，本阶段没用）
		 *   [1] 0x00000000 0       公共 R/W 段（v2 里留空）
		 *   [2] 0x6ffff000 0x7f000  peer0（root）的输出段 = 我们的 RX（只读）
		 *   [3] 0x7007e000 0x7f000  peer1（我们）的输出段 = 我们的 TX（可写）
		 * 宏来自 jailhouse/include/jailhouse/cell-config.h：
		 *   dev_id=1 → [3] 带 WRITE，[2] 只读；dev_id=0 时镜像相反。
		 * ★ 这些段带 JAILHOUSE_MEM_ROOTSHARED，cell_create 时 hypervisor
		 *   不会把它们从 root cell 里 unmap（hypervisor/control.c），于是两边都能访问。
		 */
		JAILHOUSE_SHMEM_NET_REGIONS(0x6fffe000, 1),
		/* [4] 内核镜像：LOADABLE 区的 virt_start 就是加载地址 = 链接地址 */
		{
			.phys_start = 0xfa000000,
			.virt_start = 0xfa000000,
			.size = 0x100000,	/* 1MB，ch1 的内核几十 KB，留足 */
			.flags = JAILHOUSE_MEM_READ | JAILHOUSE_MEM_WRITE |
				 JAILHOUSE_MEM_EXECUTE | JAILHOUSE_MEM_LOADABLE,
		},
		/* [5] 内核 RAM：栈/bss 之后的堆、ch4+ 的帧分配器 */
		{
			.phys_start = 0xfa100000,
			.virt_start = 0xfa100000,
			.size = 0x1f00000,	/* 31MB */
			.flags = JAILHOUSE_MEM_READ | JAILHOUSE_MEM_WRITE |
				 JAILHOUSE_MEM_EXECUTE,
		},
		/* [6] mini UART：与 root cell / hypervisor 共用（ROOTSHARED） */
		{
			.phys_start = 0xfe215040,
			.virt_start = 0xfe215040,
			.size = 0x40,
			.flags = JAILHOUSE_MEM_READ | JAILHOUSE_MEM_WRITE |
				 JAILHOUSE_MEM_IO | JAILHOUSE_MEM_IO_8 |
				 JAILHOUSE_MEM_IO_32 | JAILHOUSE_MEM_ROOTSHARED,
		},
		/* [7] 通信区（ch2+ 报 cell 状态） */
		{
			.virt_start = 0xfc000000,
			.size = 0x1000,
			.flags = JAILHOUSE_MEM_READ | JAILHOUSE_MEM_WRITE |
				 JAILHOUSE_MEM_COMM_REGION,
		},
	},
};
