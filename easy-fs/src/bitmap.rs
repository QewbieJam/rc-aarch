//! 位图（ch6）—— 对照 upstream 的 `easy-fs/src/bitmap.rs`
//!
//! inode 位图和数据位图共用这个实现：每个 bit 表示"这个 inode/块有没有被占用"。
//! 位图本身也存放在磁盘块里（`[u64; 64]` = 512 字节 = 4096 个 bit）。

use super::layout::BlockBuf;
use super::{BlockDevice, BLOCK_SZ};

/// 一个位图块：512 字节 = 64 个 u64 = 4096 个 bit
type BitmapBlock = [u64; 64];
/// 一块能表示多少个 bit
const BLOCK_BITS: usize = BLOCK_SZ * 8;

pub struct Bitmap {
    start_block_id: usize,
    blocks: usize,
}

/// 把"第 bit 个"拆成 (第几块位图, 第几个 u64, u64 里第几位)
fn decomposition(mut bit: usize) -> (usize, usize, usize) {
    let block_pos = bit / BLOCK_BITS;
    bit %= BLOCK_BITS;
    (block_pos, bit / 64, bit % 64)
}

impl Bitmap {
    pub const fn new(start_block_id: usize, blocks: usize) -> Self {
        Self {
            start_block_id,
            blocks,
        }
    }

    /// 分配一个 bit（返回它的下标），全满返回 None
    pub fn alloc(&self, dev: &dyn BlockDevice) -> Option<usize> {
        for block_id in 0..self.blocks {
            let mut buf = BlockBuf::zeroed();
            dev.read_block(self.start_block_id + block_id, &mut buf.0);
            let bitmap = unsafe { &mut *buf.as_mut_ptr::<BitmapBlock>() };
            // 找一个还有 0 的 u64，用 trailing_ones 找最低位的 0
            let found = bitmap
                .iter()
                .enumerate()
                .find(|(_, bits)| **bits != u64::MAX)
                .map(|(i, bits)| (i, bits.trailing_ones() as usize));
            if let Some((i, inner)) = found {
                bitmap[i] |= 1u64 << inner;
                dev.write_block(self.start_block_id + block_id, &buf.0);
                return Some(block_id * BLOCK_BITS + i * 64 + inner);
            }
        }
        None
    }

    /// 释放一个 bit
    pub fn dealloc(&self, dev: &dyn BlockDevice, bit: usize) {
        let (block_pos, bits64_pos, inner_pos) = decomposition(bit);
        let mut buf = BlockBuf::zeroed();
        dev.read_block(self.start_block_id + block_pos, &mut buf.0);
        let bitmap = unsafe { &mut *buf.as_mut_ptr::<BitmapBlock>() };
        assert!(
            bitmap[bits64_pos] & (1u64 << inner_pos) > 0,
            "释放了一个本来就没被占用的 bit {}",
            bit
        );
        bitmap[bits64_pos] -= 1u64 << inner_pos;
        dev.write_block(self.start_block_id + block_pos, &buf.0);
    }

    /// 最多能表示多少个 bit
    pub fn maximum(&self) -> usize {
        self.blocks * BLOCK_BITS
    }
}
