use std::time::Instant;
use shell::shell_channel::BlockApplied;
use crate::handlers::handler_messages::{BlockApplicationMessage, BlockInfo};
use tezos_encoding::hash::{HashEncoding, HashType};

pub struct ApplicationMonitor {
    total_applied: usize,
    current_applied: usize,
    last_applied_block: Option<BlockApplied>,
    first_update: Instant,
    last_update: Instant,
}

impl ApplicationMonitor {
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            total_applied: 0,
            current_applied: 0,
            last_applied_block: None,
            first_update: now.clone(),
            last_update: now,
        }
    }

    pub fn block_was_applied(&mut self, block_info: BlockApplied) {
        self.total_applied += 1;
        self.current_applied += 1;
        self.last_applied_block = Some(block_info);
    }

    pub fn avg_speed(&self) -> f32 {
        self.total_applied as f32 / (self.first_update.elapsed().as_secs_f32() / 60f32)
    }

    pub fn current_speed(&self) -> f32 {
        self.current_applied as f32 / (self.last_update.elapsed().as_secs_f32() / 60f32)
    }

    pub fn snapshot(&mut self) -> BlockApplicationMessage {
        let last_block = if let Some(ref block) = self.last_applied_block {
            Some(BlockInfo {
                hash: HashEncoding::new(HashType::BlockHash).bytes_to_string(&block.hash),
                level: block.level,
            })
        } else {
            None
        };

        let ret = BlockApplicationMessage {
            current_application_speed: self.current_speed(),
            average_application_speed: self.avg_speed(),
            last_applied_block: last_block,
        };

        self.current_applied = 0;
        self.last_update = Instant::now();
        ret
    }
}