/*
* Copyright 2019 Comcast Cable Communications Management, LLC
*
* Licensed under the Apache License, Version 2.0 (the "License");
* you may not use this file except in compliance with the License.
* You may obtain a copy of the License at
*
* http://www.apache.org/licenses/LICENSE-2.0
*
* Unless required by applicable law or agreed to in writing, software
* distributed under the License is distributed on an "AS IS" BASIS,
* WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
* See the License for the specific language governing permissions and
* limitations under the License.
*
* SPDX-License-Identifier: Apache-2.0
*/

//! End-to-end runtime tests.
//!
//! These tests initialize the full DPDK EAL environment, so they can only
//! run on a Linux machine with DPDK available (such as the CI sandbox).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use capsule::batch::{Batch, Pipeline, Poll};
use capsule::config::RuntimeConfig;
use capsule::{PortQueue, Runtime};

/// Builds a runtime against a `net_null0` virtual device, installs a
/// pipeline and a periodic task, and verifies that `execute` runs the
/// tasks until the configured `duration` expires.
#[test]
fn runtime_executes_until_duration_expires() {
    let config: RuntimeConfig = toml::from_str(
        r#"
        app_name = "runtime-test"
        master_core = 0
        duration = 2

        [mempool]
            capacity = 1023

        [[ports]]
            name = "eth0"
            device = "net_null0"
            cores = [0]
        "#,
    )
    .unwrap();

    // counts the periodic task invocations to verify that the timers are
    // firing on the master core while the runtime is executing.
    let ticks = Arc::new(AtomicUsize::new(0));
    let task_ticks = ticks.clone();

    let start = Instant::now();

    Runtime::build(config)
        .unwrap()
        .add_pipeline_to_port("eth0", |q: PortQueue| Poll::new(q.clone()).send(q))
        .unwrap()
        .add_periodic_task_to_core(
            0,
            move || {
                task_ticks.fetch_add(1, Ordering::SeqCst);
            },
            Duration::from_millis(100),
        )
        .unwrap()
        .execute()
        .unwrap();

    let elapsed = start.elapsed();
    assert!(
        elapsed >= Duration::from_secs(2),
        "execute returned before the duration expired: {:?}",
        elapsed
    );
    assert!(
        elapsed < Duration::from_secs(10),
        "execute took too long to finish: {:?}",
        elapsed
    );

    let ran = ticks.load(Ordering::SeqCst);
    assert!(
        ran > 0,
        "periodic task never ran during the {:?} execution.",
        elapsed
    );
}
