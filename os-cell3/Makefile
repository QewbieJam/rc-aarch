# rc-aarch 阶段1 构建脚本
TARGET  := aarch64-unknown-none
MODE    ?= release
ELF     := target/$(TARGET)/$(MODE)/rc-aarch-os1
QEMU    := qemu-system-aarch64
QEMU_ARGS := -machine virt -cpu cortex-a72 -nographic -kernel $(ELF)

ifeq ($(MODE),release)
CARGO_ARGS := --release
endif

build:
	@# ⚠️ 改动 linker.ld 不会被 cargo 感知（它不是 cargo 跟踪的依赖）
	@#    所以每次构建前 touch 一下源文件，强制重新链接
	@touch src/main.rs
	cargo build $(CARGO_ARGS)

run: build
	$(QEMU) $(QEMU_ARGS)

disasm: build
	rust-objdump -d $(ELF) | head -60

size: build
	rust-objdump -h $(ELF)

clean:
	cargo clean

.PHONY: build run disasm size clean
