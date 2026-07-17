//! Sequential launcher / dependency orderer (F7/F8, architecture §3.5; detailed design §3).
//!
//! Builds a dependency graph from `repository_dependencies`, computes a valid launch order via
//! topological sort (Kahn's algorithm), rejects cycles before starting anything, then drives the
//! process manager one repo at a time with the configured `launch_delay_ms` between starts.
//! v1 approximates readiness with the fixed delay (health checks are deferred, R1).
//!
//! TODO: implement `build_launch_order()` and `run_sequential_launch()`.
