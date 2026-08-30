//! Process-local, in-memory cache for per-repository git status (spec 2026-07-31). Not
//! persisted to disk and not shared across app launches - it exists only to avoid redundant
//! `git` subprocess spawns when the frontend re-checks status shortly after the last check
//! (e.g. switching back to the Project tab).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::commands::RepoGitStatus;

pub const GIT_STATUS_CACHE_TTL: Duration = Duration::from_secs(5);

type CacheKey = (i64, PathBuf);

#[derive(Default)]
pub struct GitStatusCache {
    entries: Mutex<HashMap<CacheKey, (Instant, RepoGitStatus)>>,
}

impl GitStatusCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(
        &self,
        repository_id: i64,
        canonical_path: &PathBuf,
        ttl: Duration,
    ) -> Option<RepoGitStatus> {
        let entries = self.entries.lock().unwrap();
        let (inserted_at, status) = entries.get(&(repository_id, canonical_path.clone()))?;
        if inserted_at.elapsed() < ttl {
            Some(status.clone())
        } else {
            None
        }
    }

    pub fn insert(&self, repository_id: i64, canonical_path: PathBuf, status: RepoGitStatus) {
        let mut entries = self.entries.lock().unwrap();
        entries.insert((repository_id, canonical_path), (Instant::now(), status));
    }

    /// Drops every cached entry for `repository_id`, regardless of the path it was cached under.
    pub fn invalidate(&self, repository_id: i64) {
        let mut entries = self.entries.lock().unwrap();
        entries.retain(|(id, _), _| *id != repository_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_status(id: i64) -> RepoGitStatus {
        RepoGitStatus {
            repository_id: id,
            branch: "main".to_string(),
            dirty: false,
            ahead: 0,
            behind: 0,
        }
    }

    #[test]
    fn returns_none_when_absent() {
        let cache = GitStatusCache::new();
        assert!(cache
            .get(1, &PathBuf::from("/x"), Duration::from_secs(60))
            .is_none());
    }

    #[test]
    fn returns_cached_value_within_ttl() {
        let cache = GitStatusCache::new();
        cache.insert(1, PathBuf::from("/x"), sample_status(1));
        let hit = cache.get(1, &PathBuf::from("/x"), Duration::from_secs(60));
        assert_eq!(hit.unwrap().branch, "main");
    }

    #[test]
    fn expired_entry_returns_none() {
        let cache = GitStatusCache::new();
        cache.insert(1, PathBuf::from("/x"), sample_status(1));
        assert!(cache.get(1, &PathBuf::from("/x"), Duration::ZERO).is_none());
    }

    #[test]
    fn different_path_is_a_cache_miss_even_for_same_repository_id() {
        let cache = GitStatusCache::new();
        cache.insert(1, PathBuf::from("/x"), sample_status(1));
        assert!(cache
            .get(1, &PathBuf::from("/y"), Duration::from_secs(60))
            .is_none());
    }

    #[test]
    fn invalidate_removes_entry_regardless_of_cached_path() {
        let cache = GitStatusCache::new();
        cache.insert(1, PathBuf::from("/x"), sample_status(1));
        cache.invalidate(1);
        assert!(cache
            .get(1, &PathBuf::from("/x"), Duration::from_secs(60))
            .is_none());
    }

    #[test]
    fn invalidate_does_not_affect_other_repository_ids() {
        let cache = GitStatusCache::new();
        cache.insert(1, PathBuf::from("/x"), sample_status(1));
        cache.insert(2, PathBuf::from("/y"), sample_status(2));
        cache.invalidate(1);
        assert!(cache
            .get(2, &PathBuf::from("/y"), Duration::from_secs(60))
            .is_some());
    }
}
