//! The property "local data is never lost" (plan run 7, section 4;
//! principle 3): random sequences of creates, edits, deletes and syncs on
//! two devices, deterministic per seed. After every step, every value a
//! device wrote and has not pushed yet (to a record it did not delete
//! itself) is still on that device: live, in a conflict copy, or in
//! Recently deleted. A pushed value may be replaced by the other device's
//! later edit, made on top of it: that is the user's action, not a loss.
//!
//! Editors (run 15, D220) read a host, stay open while other steps run
//! (pulls among them) and then save one member from what they read: the
//! members the editor left alone keep the version the device holds at the
//! save, and a value of the same member that the save overwrites is kept
//! in a conflict copy.

mod support;

use std::collections::HashMap;

use serde_json::{Value, json};
use shellpp_core::CoreError;
use shellpp_core::service::SignInOutcome;
use shellpp_core::service::dto::{RecordBase, RecordType};
use support::*;
use uuid::Uuid;

/// The members the steps write, each with values unique to the run.
const MEMBERS: [&str; 2] = ["label", "notes"];

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

/// The written members of a data object.
fn values_of(data: &serde_json::Map<String, Value>) -> impl Iterator<Item = String> + '_ {
    MEMBERS
        .iter()
        .filter_map(|member| data.get(*member).and_then(Value::as_str))
        .map(str::to_owned)
}

/// Every value a device can still find: live hosts, the losing data of
/// conflict copies, and Recently deleted.
fn findable(client: &Client) -> Vec<String> {
    let mut found: Vec<String> = client
        .views(&[RecordType::Host])
        .iter()
        .flat_map(|view| {
            let data = client.data(view.record_id());
            values_of(data.as_object().expect("object")).collect::<Vec<_>>()
        })
        .collect();
    for conflict in client.core.sync_conflicts().expect("conflicts") {
        if let Some(data) = conflict.losing_data.as_ref() {
            found.extend(values_of(data));
        }
    }
    for entry in client.core.recently_deleted_list().expect("trash") {
        found.push(entry.title);
        if let Some(data) = entry.data.as_ref() {
            found.extend(values_of(data));
        }
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

/// The data of a live host on `client`, `None` when it is not live there.
fn live_data(client: &Client, id: Uuid) -> Option<Value> {
    live_ids(client).contains(&id).then(|| client.data(id))
}

/// Saves an open editor of `id` that read `base`, with `member` set to
/// `value`, and checks the partial save: the other members are the ones
/// the device holds now, and an overwritten value of `member` is in a
/// conflict copy. Returns `false` when the record could not be written.
fn save_editor(
    client: &Client,
    id: Uuid,
    base: &RecordBase,
    member: &str,
    value: &str,
    context: &str,
) -> bool {
    let before = live_data(client, id);
    let mut form = Value::Object(base.data.clone());
    form[member] = Value::String(value.to_owned());
    match client.save_from(id, RecordType::Host, base, form) {
        Ok(_) => {}
        // Deleted and gone from Recently deleted: nothing to write onto.
        Err(CoreError::NotFound) if before.is_none() => return false,
        Err(error) => panic!("{context}: editor save failed: {}", error.code()),
    }
    let after = client.data(id);
    assert_eq!(after[member], value, "{context}: the edit is saved");
    if let Some(before) = before {
        for other in MEMBERS.iter().filter(|other| **other != member) {
            assert_eq!(
                after[*other], before[*other],
                "{context}: {other} the editor left alone was changed"
            );
        }
        let overwritten = &before[member];
        if let Some(text) = overwritten.as_str()
            && Some(overwritten) != base.data.get(member)
            && text != value
        {
            assert!(
                findable(client).iter().any(|found| found == text),
                "{context}: the overwritten {member} {text} is in no conflict copy"
            );
        }
    }
    true
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
    // Per device: (record, member) → the latest value it wrote there.
    let mut written: [HashMap<(Uuid, &str), String>; 2] = [HashMap::new(), HashMap::new()];
    // Per device: an open editor, with the version it read.
    let mut editors: [Option<(Uuid, RecordBase)>; 2] = [None, None];
    let mut rng = Rng(seed);
    let mut counter = 0;
    for step in 0..steps {
        let mut synced = None;
        let who = rng.below(2);
        let device = devices[who];
        let ids = live_ids(device);
        counter += 1;
        let value = format!("{}-{seed}-{counter}", ["A", "B"][who]);
        let member = MEMBERS[rng.below(MEMBERS.len())];
        match rng.below(12) {
            0..=1 => {
                let notes = format!("{value}-n");
                let id = device.save(
                    None,
                    RecordType::Host,
                    json!({
                        "address": format!("h{counter}.example.com"),
                        "label": value,
                        "notes": notes,
                    }),
                );
                written[who].insert((id, "label"), value);
                written[who].insert((id, "notes"), notes);
            }
            2..=3 if !ids.is_empty() => {
                // A whole write on the current data (an import, a move).
                let id = ids[rng.below(ids.len())];
                let mut data = device.data(id);
                data[member] = Value::String(value.clone());
                device.save(Some(id), RecordType::Host, data);
                written[who].insert((id, member), value);
            }
            4 if !ids.is_empty() => {
                // The user's own deletion: nothing to keep for it.
                let id = ids[rng.below(ids.len())];
                device.delete(id);
                written[who].retain(|(record, _), _| *record != id);
            }
            5 if !ids.is_empty() && editors[who].is_none() => {
                let id = ids[rng.below(ids.len())];
                editors[who] = Some((id, device.base(id)));
            }
            6..=7 if editors[who].is_some() => {
                let (id, base) = editors[who].take().expect("open editor");
                let context = format!("seed {seed}, step {step}, device {who}");
                if save_editor(device, id, &base, member, &value, &context) {
                    written[who].insert((id, member), value);
                }
            }
            _ => {
                if device.core.sync_now().is_ok() {
                    synced = Some(who);
                }
            }
        }
        for (index, client) in devices.iter().enumerate() {
            let found = findable(client);
            for ((id, member), value) in &written[index] {
                assert!(
                    found.contains(value),
                    "seed {seed}, step {step}: device {index} lost {member} {value} of {id}"
                );
            }
        }
        // After a successful sync, the values still live there were pushed.
        if let Some(who) = synced {
            let live: Vec<String> = live_ids(devices[who])
                .into_iter()
                .flat_map(|id| {
                    let data = devices[who].data(id);
                    values_of(data.as_object().expect("object")).collect::<Vec<_>>()
                })
                .collect();
            written[who].retain(|_, value| !live.contains(value));
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
