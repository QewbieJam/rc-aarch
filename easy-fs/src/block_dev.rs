//! 块设备抽象（ch6）—— 对照 upstream 的 `easy-fs/src/block_dev.rs`
//!
//! 上层（easy-fs）只认这个 trait；具体是 virtio-blk、SD 卡还是我们的 RAM 磁盘，
//! 由使用方决定。

/// 一个"能按块读写"的设备
pub trait BlockDevice: Send + Sync {
    /// 把第 `block_id` 块读进 `buf`
    fn read_block(&self, block_id: usize, buf: &mut [u8]);
    /// 把 `buf` 写进第 `block_id` 块
    fn write_block(&self, block_id: usize, buf: &[u8]);
}
