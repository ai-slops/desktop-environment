use serde_json::Value;
use std::collections::{BTreeMap, VecDeque};
use window_manager_core::{Error, ErrorCode, Id, Result};

const MAX_BYTES: usize = 8 * 1024 * 1024;

#[derive(Default)]
pub struct ReplayCache {
    entries: BTreeMap<Id, (Vec<u8>, Value, usize)>,
    order: VecDeque<Id>,
    bytes: usize,
}

impl ReplayCache {
    pub fn lookup(&self, id: &str, fingerprint: &[u8]) -> Result<Option<Value>> {
        self.entries.get(id).map_or(Ok(None), |(prior, response, _)| {
            if prior == fingerprint {
                Ok(Some(response.clone()))
            } else {
                Err(Error::new(
                    ErrorCode::InvalidConfiguration,
                    "Command ID was already used with different arguments",
                    id,
                ))
            }
        })
    }

    pub fn insert(&mut self, id: Id, fingerprint: Vec<u8>, response: &Value) {
        let size =
            serde_json::to_vec(response).map_or(MAX_BYTES, |bytes| bytes.len()) + fingerprint.len();
        let response = if size > MAX_BYTES {
            serde_json::json!({"id":id,"error":Error::new(ErrorCode::StaleRevision,"Original command already finished; its response exceeds the replay budget. Effects suppressed; obtain a fresh snapshot.",&id),"cursor":response["cursor"]})
        } else {
            response.clone()
        };
        let size = serde_json::to_vec(&response).map_or(MAX_BYTES, |bytes| bytes.len())
            + fingerprint.len();
        while self.order.len() >= 256 || self.bytes.saturating_add(size) > MAX_BYTES {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            if let Some((_, _, bytes)) = self.entries.remove(&oldest) {
                self.bytes = self.bytes.saturating_sub(bytes);
            }
        }
        self.bytes += size;
        self.order.push_back(id.clone());
        self.entries.insert(id, (fingerprint, response, size));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn oversized_response_retains_a_receipt_that_suppresses_reexecution() -> Result<()> {
        let mut cache = ReplayCache::default();
        cache.insert("large".into(), b"action".to_vec(), &serde_json::json!({"result":"x".repeat(MAX_BYTES), "cursor":{"epoch":"a","sequence":1}}));
        let receipt = cache
            .lookup("large", b"action")?
            .ok_or_else(|| Error::new(ErrorCode::TargetMissing, "receipt missing", "fixture"))?;
        assert_eq!(receipt["error"]["code"], "STALE_REVISION");
        assert!(cache.bytes < MAX_BYTES);
        Ok(())
    }

    #[test]
    fn retries_replay_original_results_and_id_collisions_do_not_change_arguments() -> Result<()> {
        let mut cache = ReplayCache::default();
        let response = serde_json::json!({"id":"x","result":{"revision":1}});
        cache.insert("x".into(), b"same request".to_vec(), &response);
        assert_eq!(cache.lookup("x", b"same request")?, Some(response));
        assert_eq!(
            cache.lookup("x", b"wider scope").err().map(|error| error.code),
            Some(ErrorCode::InvalidConfiguration)
        );
        for index in 0..257 {
            cache.insert(index.to_string(), Vec::new(), &serde_json::json!({"result":true}));
        }
        assert!(cache.lookup("x", b"same request")?.is_none());
        assert!(cache.entries.len() <= 256);
        Ok(())
    }
}
