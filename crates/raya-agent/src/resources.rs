//! Bounded resource manager.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::Duration;

use raya_core::{Config, ResourcesConfig};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::time::timeout;

/// Configurable resource limits.
#[derive(Debug, Clone)]
pub struct ResourceLimits {
    pub max_parallel_tools: u32,
    pub max_parallel_agents: u32,
    pub max_processes: u32,
    pub max_iterations: u32,
    pub max_tool_calls: u32,
    pub max_tokens: u64,
}

impl ResourceLimits {
    pub fn from_config(config: &Config) -> Self {
        Self {
            max_parallel_tools: config.resources.max_parallel_tools,
            max_parallel_agents: config
                .resources
                .max_parallel_agents
                .max(config.subagent.max_parallel),
            max_processes: config.resources.max_processes,
            max_iterations: config.agent.max_iterations,
            max_tool_calls: config.agent.max_tool_calls,
            max_tokens: config.context.max_tokens,
        }
    }

    pub fn from_resources(
        r: &ResourcesConfig,
        iterations: u32,
        tool_calls: u32,
        tokens: u64,
    ) -> Self {
        Self {
            max_parallel_tools: r.max_parallel_tools,
            max_parallel_agents: r.max_parallel_agents,
            max_processes: r.max_processes,
            max_iterations: iterations,
            max_tool_calls: tool_calls,
            max_tokens: tokens,
        }
    }
}

/// Tracks and bounds concurrency / budgets for a task or runtime.
pub struct ResourceManager {
    limits: ResourceLimits,
    tool_sem: Arc<Semaphore>,
    process_sem: Arc<Semaphore>,
    agent_sem: Arc<Semaphore>,
    iterations: AtomicU32,
    tool_calls: AtomicU32,
    tokens_used: AtomicU64,
}

pub struct ResourcePermit {
    _permit: OwnedSemaphorePermit,
}

impl ResourceManager {
    pub fn new(limits: ResourceLimits) -> Self {
        Self {
            tool_sem: Arc::new(Semaphore::new(limits.max_parallel_tools as usize)),
            process_sem: Arc::new(Semaphore::new(limits.max_processes as usize)),
            agent_sem: Arc::new(Semaphore::new(limits.max_parallel_agents as usize)),
            iterations: AtomicU32::new(0),
            tool_calls: AtomicU32::new(0),
            tokens_used: AtomicU64::new(0),
            limits,
        }
    }

    pub fn limits(&self) -> &ResourceLimits {
        &self.limits
    }

    pub async fn acquire_tool(&self) -> Result<ResourcePermit, String> {
        let permit = self
            .tool_sem
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| "tool semaphore closed".to_string())?;
        Ok(ResourcePermit { _permit: permit })
    }

    pub async fn try_acquire_tool_bounded(&self, wait: Duration) -> Result<ResourcePermit, String> {
        match timeout(wait, self.tool_sem.clone().acquire_owned()).await {
            Ok(Ok(permit)) => Ok(ResourcePermit { _permit: permit }),
            Ok(Err(_)) => Err("tool semaphore closed".into()),
            Err(_) => Err("tool concurrency limit reached".into()),
        }
    }

    pub async fn acquire_process(&self) -> Result<ResourcePermit, String> {
        let permit = self
            .process_sem
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| "process semaphore closed".to_string())?;
        Ok(ResourcePermit { _permit: permit })
    }

    pub async fn acquire_agent(&self) -> Result<ResourcePermit, String> {
        let permit = self
            .agent_sem
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| "agent semaphore closed".to_string())?;
        Ok(ResourcePermit { _permit: permit })
    }

    pub fn bump_iteration(&self) -> Result<u32, String> {
        let n = self.iterations.fetch_add(1, Ordering::SeqCst) + 1;
        if n > self.limits.max_iterations {
            return Err(format!(
                "max iterations exceeded ({})",
                self.limits.max_iterations
            ));
        }
        Ok(n)
    }

    pub fn bump_tool_call(&self) -> Result<u32, String> {
        let n = self.tool_calls.fetch_add(1, Ordering::SeqCst) + 1;
        if n > self.limits.max_tool_calls {
            return Err(format!(
                "max tool calls exceeded ({})",
                self.limits.max_tool_calls
            ));
        }
        Ok(n)
    }

    pub fn add_tokens(&self, n: u64) -> Result<u64, String> {
        let total = self.tokens_used.fetch_add(n, Ordering::SeqCst) + n;
        if total > self.limits.max_tokens.saturating_mul(4) {
            // hard ceiling: 4x context budget for cumulative task tokens
            return Err("token budget exceeded".into());
        }
        Ok(total)
    }

    pub fn iterations(&self) -> u32 {
        self.iterations.load(Ordering::SeqCst)
    }

    pub fn tool_calls(&self) -> u32 {
        self.tool_calls.load(Ordering::SeqCst)
    }

    pub fn tokens_used(&self) -> u64 {
        self.tokens_used.load(Ordering::SeqCst)
    }

    pub fn available_tools(&self) -> usize {
        self.tool_sem.available_permits()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits(n: u32) -> ResourceLimits {
        ResourceLimits {
            max_parallel_tools: n,
            max_parallel_agents: 3,
            max_processes: 8,
            max_iterations: 12,
            max_tool_calls: 40,
            max_tokens: 40_000,
        }
    }

    #[tokio::test]
    async fn tool_semaphore_bounds() {
        let rm = ResourceManager::new(limits(1));
        let p1 = rm.acquire_tool().await.unwrap();
        let err = rm.try_acquire_tool_bounded(Duration::from_millis(50)).await;
        assert!(err.is_err());
        drop(p1);
        let p2 = rm.acquire_tool().await.unwrap();
        drop(p2);
    }

    #[tokio::test]
    async fn iteration_limit() {
        let mut l = limits(4);
        l.max_iterations = 2;
        let rm = ResourceManager::new(l);
        assert!(rm.bump_iteration().is_ok());
        assert!(rm.bump_iteration().is_ok());
        assert!(rm.bump_iteration().is_err());
    }
}
