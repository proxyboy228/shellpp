//! The property "local data is never lost" (plan run 7, section 4;
//! principle 3): random sequences of creates, edits, deletes and syncs on
//! two devices, deterministic per seed. After every step, every value a
//! device wrote and has not pushed yet (to a record it did not delete
//! itself) is still on that device: live, in a conflict copy, or in
//! Recently deleted. A pushed value may be replaced by the other device's
//! later edit, made on top of it: that is the user's action, not a loss.

mod support;

use std::collections::HashMap;

use serde_json::{Value, json};
use shellpp_core::service::SignInOutcome;
use shellpp_core::service::dto::RecordType;
use support::*;
use uuid::Uuid;

/// xorshift64*: deterministic, no dependency.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// Every label a device can still find: live hosts, the losing data of
/// conflict copies, and Recently deleted titles.
fn findable(client: &Client) -> Vec<String> {
    let mut found: Vec<String> = client
        .views(&[RecordType::Host])
        .iter()
        .filter_map(|view| {
            client.data(view.record_id())["label"]
                .as_str()
                .map(str::to_owned)
        })
        .collect();
    for conflict in client.core.sync_conflicts().expect("conflicts") {
        if let Some(label) = conflict
            .losing_data
            .as_ref()
            .and_then(|data| data.get("label"))
            .and_then(Value::as_str)
        {
            found.push(label.to_owned());
        }
    }
    for entry in client.core.recently_deleted_list().expect("trash") {
        found.push(entry.title);
    }
    found
}

fn live_ids(client: &Client) -> Vec<Uuid> {
    client
        .views(&[RecordType::Host])
        .iter()
        .map(|view| view.record_id())
        .collect()
}

fn run(seed: u64, steps: usize) {
    let server = Server::start();
    let password = new_password();
    let email = email();
    let a = Client::local("A", &password);
    a.register(&server, &email, &password);
    let b = Client::empty("B");
    assert!(matches!(
        b.sign_in(&server, &email, &password),
        SignInOutcome::Done(_)
    ));
    let devices = [&a, &b];
    // Per device: record → the latest label it wrote there.
    let mut written: [HashMap<Uuid, String>; 2] = [HashMap::new(), HashMap::new()];
    let mut rng = Rng(seed);
    let mut counter = 0;
    for step in 0..steps {
        let mut synced = None;
        let who = rng.below(2);
        let device = devices[who];
        let ids = live_ids(device);
        counter += 1;
        let label = format!("{}-{seed}-{counter}", ["A", "B"][who]);
        match rng.below(10) {
            0..=1 => {
                let id = device.save(
                    None,
                    RecordType::Host,
                    json!({"address": format!("h{counter}.example.com"), "label": label}),
                );
                written[who].insert(id, label);
            }
            2..=4 if !ids.is_empty() => {
                let id = ids[rng.below(ids.len())];
                let mut data = device.data(id);
                data["label"] = Value::String(label.clone());
                device.save(Some(id), RecordType::Host, data);
                written[who].insert(id, label);
            }
            5 if !ids.is_empty() => {
                // The user's own deletion: nothing to keep for it.
                let id = ids[rng.below(ids.len())];
                device.delete(id);
                written[who].remove(&id);
            }
            _ => {
                if device.core.sync_now().is_ok() {
                    synced = Some(who);
                }
            }
        }
        for (index, client) in devices.iter().enumerate() {
            let found = findable(client);
            for (id, label) in &written[index] {
                assert!(
                    found.contains(label),
                    "seed {seed}, step {step}: device {index} lost {label} of {id}"
                );
            }
        }
        // After a successful sync, the values still live there were pushed.
        if let Some(who) = synced {
            let live: Vec<String> = live_ids(devices[who])
                .into_iter()
                .filter_map(|id| devices[who].data(id)["label"].as_str().map(str::to_owned))
                .collect();
            written[who].retain(|_, label| !live.contains(label));
        }
    }
    // Both converge after a final exchange.
    for _ in 0..2 {
        a.sync();
        b.sync();
    }
    assert_eq!(a.addresses(), b.addresses(), "seed {seed}: converged");
}

/// `SHELLPP_PROPERTY_SEEDS` (how many seeds, from the four fixed ones on)
/// and `SHELLPP_PROPERTY_STEPS` (steps per seed) make a longer run of the
/// verification runs; the default is four seeds of 60 steps.
#[test]
fn local_data_is_never_lost_under_random_operations() {
    let number = |name: &str, default: u64| {
        std::env::var(name)
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(default)
    };
    let seeds = number("SHELLPP_PROPERTY_SEEDS", 4);
    let steps = usize::try_from(number("SHELLPP_PROPERTY_STEPS", 60)).unwrap_or(60);
    let fixed = [0x5eed_0001_u64, 0x5eed_0002, 0x5eed_0003, 0x0bad_c0de];
    for index in 0..seeds {
        let seed = fixed
            .get(index as usize)
            .copied()
            .unwrap_or(0x5eed_1000 + index);
        run(seed, steps);
    }
}
