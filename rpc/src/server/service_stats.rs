use std::cmp::Reverse;
use std::collections::{HashMap, BinaryHeap};
use std::sync::{MutexGuard, RwLock, Mutex};
use tezos_context::channel::ContextAction;
use crate::rpc_actor::RpcCollectedStateRef;
use storage::persistent::PersistentStorage;
use crypto::hash::HashType;
use storage::{BlockStorage, ContextActionStorage, BlockStorageReader};
use rayon::iter::ParallelIterator;
use rayon::iter::IntoParallelRefIterator;
use serde::{Deserialize, Serialize};
use crate::server::service::get_block_actions_by_hash;

#[derive(Serialize, Deserialize)]
pub struct ActionTypeStats {
    total_time: f64,
    total_actions: u32,
}

#[derive(Serialize, Deserialize)]
pub struct ActionStats {
    total_time: f64,
    total_actions: u32,
    action_type_stats: HashMap<String, ActionTypeStats>,
}

fn add_action<'a>(
    stats: &mut MutexGuard<HashMap<&'a str, ActionStats>>,
    action_name: &'a str,
    key: Option<&Vec<String>>,
    time: f64
) {
    let mut action_stats = stats
        .entry(action_name).or_insert(ActionStats {
        action_type_stats: HashMap::new(), total_time: 0f64, total_actions: 0
    });
    action_stats.total_time += time;
    action_stats.total_actions += 1;

    match key {
        Some(key) => {
            if let Some(first) = key.get(0) {
                let action_type = if first == "data" {
                    if let Some(second) = key.get(1) {
                        second
                    } else {
                        first
                    }
                } else { first };
                let actions_type_stats = action_stats.action_type_stats
                    .entry(action_type.clone())
                    .or_insert(ActionTypeStats { total_time: 0f64, total_actions: 0 });
                actions_type_stats.total_time += time;
                actions_type_stats.total_actions += 1;
            };
        },
        None => {},
    }
}

struct TopN<T: Ord> {
    data: RwLock<BinaryHeap<Reverse<T>>>, // the Reverse makes it a min-heap
    max: usize,
}

impl<T: Ord + Clone> TopN<T> {
    pub fn new(max: usize) -> TopN<T> { return TopN { data: RwLock::new(BinaryHeap::new()), max } }

    pub fn add(&self, val: &T) {
        let heap_len;
        let should_add;
        {
            let data = self.data.read().expect("Unable to get a read-only lock!");
            heap_len = data.len();
            should_add = if data.is_empty() { true } else { data.peek().unwrap().0 < *val };
        } // drop the read lock

        if heap_len < self.max { // heap not full, keep adding
            let mut data = self.data.write().expect("Unable to get a write lock!");
            return data.push(Reverse(val.clone()));
        }

        if should_add { // a new larger element, add it to the heap
            let mut data_write = self.data.write().expect("Unable to get a write lock!");
            data_write.pop();
            data_write.push(Reverse(val.clone()));
        };
    }
}

#[derive(Serialize)]
pub struct StatsResponse<'a> {
    fat_tail: Vec<ContextAction>,
    stats: HashMap<&'a str, ActionStats>,
}

pub(crate) fn compute_storage_stats<'a>(
    _state: &RpcCollectedStateRef,
    from_block: &str,
    persistent_storage: &PersistentStorage
) -> Result<StatsResponse<'a>, failure::Error> {
    let context_action_storage = ContextActionStorage::new(persistent_storage);
    let block_storage = BlockStorage::new(persistent_storage);
    let stats: Mutex<HashMap<&str, ActionStats>> = Mutex::new(HashMap::new());
    let fat_tail: TopN<ContextAction> = TopN::new(100);

    let blocks = block_storage.get_multiple_without_json(
        &HashType::BlockHash.string_to_bytes(from_block).unwrap(), std::usize::MAX)?;
    blocks.par_iter().for_each(|block| {
        let actions = get_block_actions_by_hash(&context_action_storage, &block.hash).expect("Failed to extract actions from a block!");
        {
            let mut stats = stats.lock().expect("Unable to lock mutex!");
            actions.iter().for_each(|action| match action {
                ContextAction::Set { key, start_time, end_time, .. } =>
                    add_action(&mut stats, "SET", Some(key), *end_time - *start_time),
                ContextAction::Delete { key, start_time, end_time, .. } =>
                    add_action(&mut stats, "DEL", Some(key), *end_time - *start_time),
                ContextAction::RemoveRecursively { key, start_time, end_time, .. } =>
                    add_action(&mut stats, "REMREC", Some(key), *end_time - *start_time),
                ContextAction::Copy { start_time, end_time, .. } =>
                    add_action(&mut stats, "COPY", None, *end_time - *start_time),
                ContextAction::Checkout { start_time, end_time, .. } =>
                    add_action(&mut stats, "CHECKOUT", None, *end_time - *start_time),
                ContextAction::Commit { start_time, end_time, .. } =>
                    add_action(&mut stats, "COMMIT", None, *end_time - *start_time),
                ContextAction::Mem { key, start_time, end_time, .. } =>
                    add_action(&mut stats, "MEM", Some(key), *end_time - *start_time),
                ContextAction::DirMem { key, start_time, end_time, .. } =>
                    add_action(&mut stats, "DIRMEM", Some(key), *end_time - *start_time),
                ContextAction::Get { key, start_time, end_time, .. } =>
                    add_action(&mut stats, "GET", Some(key), *end_time - *start_time),
                ContextAction::Fold { key, start_time, end_time, .. } =>
                    add_action(&mut stats, "FOLD", Some(key), *end_time - *start_time),
                ContextAction::Shutdown => {}
            });
        } // drop the mutex here

        actions.par_iter().for_each(|action| fat_tail.add(action));
    });

    Ok(StatsResponse {
        fat_tail: remove_values(fat_tail_vec(fat_tail)),
        stats: stats.into_inner().expect("Unable to access the stat contents of mutex!"),
    })
}

fn remove_values(mut actions: Vec<ContextAction>) -> Vec<ContextAction> {
    actions.iter_mut().for_each(|action| match action {
        ContextAction::Set { ref mut value, ref mut value_as_json, .. } => {
            value.resize(0, 0);
            *value_as_json = None;
        },
        _ => {}
    });
    actions
}

fn fat_tail_vec(fat_tail: TopN<ContextAction>) -> Vec<ContextAction> {
    fat_tail.data
        .into_inner()
        .expect("Unable to get fat tail lock data!")
        .into_sorted_vec()
        .drain(0..)
        .map(move |reverse_wrapped| reverse_wrapped.0)
        .collect()
}
