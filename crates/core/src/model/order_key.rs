//! Manual order keys (docs/data-model.md, section 18.1; D23).
//!
//! A key is 1 to 64 characters from `0-9A-Za-z` that does not end in `0`,
//! read as a base-62 fraction `0.d1d2...`. Keys compare by bytes (ASCII order
//! equals digit order); equal keys are ordered by `record_id`; items without a
//! key sort after keyed items, by `record_id`. A key between two others is
//! their midpoint, shortened to the fewest digits that stay strictly between
//! them. When no key of at most 64 characters fits, the container is
//! rebalanced with evenly spaced short keys (one write per item).

use std::cmp::Ordering;

use uuid::Uuid;

/// The digits in ascending order.
pub const ALPHABET: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
/// Maximum key length.
pub const MAX_LEN: usize = 64;

const BASE: u32 = 62;
/// Working width: 64 digits per key plus one for the halving.
const WIDTH: usize = MAX_LEN + 2;

fn digit_value(byte: u8) -> Option<u32> {
    match byte {
        b'0'..=b'9' => Some(u32::from(byte - b'0')),
        b'A'..=b'Z' => Some(u32::from(byte - b'A') + 10),
        b'a'..=b'z' => Some(u32::from(byte - b'a') + 36),
        _ => None,
    }
}

/// A well-formed key.
pub fn is_valid(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= MAX_LEN
        && key.bytes().all(|b| digit_value(b).is_some())
        && !key.ends_with('0')
}

/// Fixed-point value: index 0 is the integer part, 1..=WIDTH the digits.
type Fixed = [u32; WIDTH + 1];

fn to_fixed(key: &str) -> Option<Fixed> {
    if !is_valid(key) {
        return None;
    }
    let mut out = [0; WIDTH + 1];
    for (index, byte) in key.bytes().enumerate() {
        out[index + 1] = digit_value(byte)?;
    }
    Some(out)
}

fn one() -> Fixed {
    let mut out = [0; WIDTH + 1];
    out[0] = 1;
    out
}

fn midpoint(a: &Fixed, b: &Fixed) -> Fixed {
    let mut sum = [0u32; WIDTH + 1];
    let mut carry = 0;
    for index in (0..=WIDTH).rev() {
        let total = a[index] + b[index] + carry;
        if index == 0 {
            sum[0] = total;
        } else {
            sum[index] = total % BASE;
            carry = total / BASE;
        }
    }
    let mut mid = [0u32; WIDTH + 1];
    let mut remainder = 0;
    for index in 0..=WIDTH {
        let current = remainder * BASE + sum[index];
        mid[index] = current / 2;
        remainder = current % 2;
    }
    mid
}

fn render(value: &Fixed, digits: usize) -> Option<String> {
    let mut text: Vec<u8> = (1..=digits)
        .map(|index| ALPHABET[value[index] as usize])
        .collect();
    while text.last() == Some(&b'0') {
        text.pop();
    }
    if text.is_empty() {
        None
    } else {
        String::from_utf8(text).ok()
    }
}

/// A key strictly between `low` and `high` (`None` = the start and the end of
/// the list). `None` when the bounds are malformed or out of order, or when no
/// key of at most 64 characters fits (rebalance the container).
pub fn between(low: Option<&str>, high: Option<&str>) -> Option<String> {
    let low = match low {
        Some(key) => to_fixed(key)?,
        None => [0; WIDTH + 1],
    };
    let high = match high {
        Some(key) => to_fixed(key)?,
        None => one(),
    };
    if low >= high {
        return None;
    }
    let mid = midpoint(&low, &high);
    for digits in 1..=MAX_LEN {
        let mut floor = [0u32; WIDTH + 1];
        floor[..=digits].copy_from_slice(&mid[..=digits]);
        if floor > low && floor < high {
            return render(&floor, digits);
        }
        let mut ceil = floor;
        let mut index = digits;
        loop {
            ceil[index] += 1;
            if ceil[index] < BASE || index == 0 {
                break;
            }
            ceil[index] = 0;
            index -= 1;
        }
        if ceil[0] == 0 && ceil > low && ceil < high {
            return render(&ceil, digits);
        }
    }
    None
}

/// `count` evenly spaced short keys in ascending order.
pub fn spaced(count: usize) -> Vec<String> {
    if count == 0 {
        return Vec::new();
    }
    let slots = count as u128 + 1;
    let mut digits = 1usize;
    let mut range: u128 = u128::from(BASE);
    while range < slots && digits < 20 {
        digits += 1;
        range *= u128::from(BASE);
    }
    (1..=count as u128)
        .map(|index| {
            let mut value = index * range / slots;
            let mut text = vec![b'0'; digits];
            for position in (0..digits).rev() {
                text[position] = ALPHABET[(value % u128::from(BASE)) as usize];
                value /= u128::from(BASE);
            }
            while text.last() == Some(&b'0') {
                text.pop();
            }
            String::from_utf8(text).unwrap_or_default()
        })
        .collect()
}

/// Manual order: keyed items by key bytes, then by `record_id`; keyless items
/// after keyed ones, by `record_id`.
pub fn compare(a: (Option<&str>, &Uuid), b: (Option<&str>, &Uuid)) -> Ordering {
    match (a.0, b.0) {
        (Some(x), Some(y)) => x.as_bytes().cmp(y.as_bytes()).then_with(|| a.1.cmp(b.1)),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => a.1.cmp(b.1),
    }
}

/// Keys to write when `inserted` records are placed at `index` of a container
/// whose current manual order (without them) is `order`. Returns the new key of
/// every inserted record and, when the container had to be rebalanced, the new
/// keys of the other records whose key changed.
pub fn place(
    order: &[(Uuid, Option<String>)],
    index: usize,
    inserted: &[Uuid],
) -> Vec<(Uuid, String)> {
    let index = index.min(order.len());
    let usable = |key: &Option<String>| {
        key.as_deref()
            .filter(|key| is_valid(key))
            .map(str::to_owned)
    };
    let lower = if index > 0 {
        Some(usable(&order[index - 1].1))
    } else {
        None
    };
    let upper = order.get(index).map(|(_, key)| usable(key));

    // A keyless item before the insertion point: only a rebalance can place it.
    let simple = !matches!(lower, Some(None));
    if simple {
        let low = lower.flatten();
        let high = upper.flatten();
        let mut previous = low;
        let mut keys = Vec::with_capacity(inserted.len());
        let mut ok = true;
        for id in inserted {
            match between(previous.as_deref(), high.as_deref()) {
                Some(key) => {
                    previous = Some(key.clone());
                    keys.push((*id, key));
                }
                None => {
                    ok = false;
                    break;
                }
            }
        }
        if ok {
            return keys;
        }
    }

    let mut full: Vec<(Uuid, Option<String>)> = order.to_vec();
    for (offset, id) in inserted.iter().enumerate() {
        full.insert(index + offset, (*id, None));
    }
    let keys = spaced(full.len());
    full.into_iter()
        .zip(keys)
        .filter(|((id, old), new)| inserted.contains(id) || old.as_deref() != Some(new.as_str()))
        .map(|((id, _), new)| (id, new))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validity() {
        assert!(is_valid("V"));
        assert!(is_valid("0V"));
        assert!(!is_valid(""));
        assert!(!is_valid("V0"));
        assert!(!is_valid("V-"));
        assert!(is_valid(&"z".repeat(64)));
        assert!(!is_valid(&"z".repeat(65)));
    }

    #[test]
    fn midpoints_use_fewest_digits() {
        assert_eq!(between(None, None).as_deref(), Some("V"));
        assert_eq!(between(Some("V"), None).as_deref(), Some("k"));
        assert_eq!(between(None, Some("V")).as_deref(), Some("F"));
        assert_eq!(between(Some("V"), Some("W")).as_deref(), Some("VV"));
        assert_eq!(between(Some("V"), Some("X")).as_deref(), Some("W"));
        assert_eq!(between(Some("z"), None).as_deref(), Some("zV"));
        assert_eq!(between(Some("W"), Some("V")), None);
        assert_eq!(between(Some("V"), Some("V")), None);
        assert_eq!(between(Some("bad!"), None), None);
    }

    #[test]
    fn repeated_insertions_stay_ordered() {
        let mut keys = vec![between(None, None).unwrap()];
        // Append many items.
        for _ in 0..200 {
            let last = keys.last().unwrap().clone();
            keys.push(between(Some(&last), None).unwrap());
        }
        for pair in keys.windows(2) {
            assert!(pair[0] < pair[1], "{} < {}", pair[0], pair[1]);
            assert!(is_valid(&pair[1]));
        }
        // Insert repeatedly right after the first item.
        let mut low = keys[0].clone();
        let high = keys[1].clone();
        for _ in 0..50 {
            let key = between(Some(&low), Some(&high)).unwrap();
            assert!(low < key && key < high);
            low = key;
        }
    }

    #[test]
    fn exhausted_gap_needs_rebalance() {
        let low = format!("V{}", "0".repeat(62)) + "1";
        let high = format!("V{}", "0".repeat(62)) + "2";
        assert_eq!(low.len(), 64);
        assert_eq!(between(Some(&low), Some(&high)), None);
    }

    #[test]
    fn spaced_keys_are_ordered_and_short() {
        let keys = spaced(5);
        assert_eq!(keys.len(), 5);
        for pair in keys.windows(2) {
            assert!(pair[0] < pair[1]);
        }
        assert!(keys.iter().all(|key| key.len() == 1 && is_valid(key)));
        let many = spaced(1000);
        assert_eq!(many.len(), 1000);
        for pair in many.windows(2) {
            assert!(pair[0] < pair[1]);
        }
        assert!(many.iter().all(|key| key.len() <= 2 && is_valid(key)));
    }

    #[test]
    fn compare_puts_keyless_last() {
        let a = Uuid::from_u128(1);
        let b = Uuid::from_u128(2);
        assert_eq!(compare((Some("V"), &b), (Some("W"), &a)), Ordering::Less);
        assert_eq!(compare((Some("V"), &b), (Some("V"), &a)), Ordering::Greater);
        assert_eq!(compare((Some("z"), &b), (None, &a)), Ordering::Less);
        assert_eq!(compare((None, &a), (None, &b)), Ordering::Less);
    }

    #[test]
    fn place_between_neighbors() {
        let a = Uuid::from_u128(1);
        let b = Uuid::from_u128(2);
        let new = Uuid::from_u128(3);
        let order = vec![(a, Some("V".to_owned())), (b, Some("k".to_owned()))];
        let keys = place(&order, 1, &[new]);
        assert_eq!(keys.len(), 1);
        assert!("V" < keys[0].1.as_str() && keys[0].1.as_str() < "k");
        let end = place(&order, 2, &[new]);
        assert!(end[0].1.as_str() > "k");
        let start = place(&order, 0, &[new]);
        assert!(start[0].1.as_str() < "V");
        let empty = place(&[], 0, &[new]);
        assert_eq!(empty, vec![(new, "V".to_owned())]);
    }

    #[test]
    fn place_rebalances_when_needed() {
        let a = Uuid::from_u128(1);
        let b = Uuid::from_u128(2);
        let c = Uuid::from_u128(3);
        let new = Uuid::from_u128(4);
        // Keyless item before the insertion point.
        let order = vec![(a, Some("V".to_owned())), (b, None), (c, None)];
        let keys = place(&order, 2, &[new]);
        let map: std::collections::HashMap<_, _> = keys.into_iter().collect();
        assert!(map.contains_key(&new));
        assert!(map.contains_key(&b));
        assert!(map.contains_key(&c));
        assert!(map[&b] < map[&new] && map[&new] < map[&c]);

        // Exhausted gap.
        let low = format!("V{}", "0".repeat(62)) + "1";
        let high = format!("V{}", "0".repeat(62)) + "2";
        let order = vec![(a, Some(low)), (b, Some(high))];
        let keys = place(&order, 1, &[new]);
        let map: std::collections::HashMap<_, _> = keys.into_iter().collect();
        assert_eq!(map.len(), 3);
        assert!(map[&a] < map[&new] && map[&new] < map[&b]);
        assert!(map.values().all(|key| key.len() == 1));

        // Inserting before a keyless item that follows keyed ones needs no rebalance.
        let order = vec![(a, Some("V".to_owned())), (b, None)];
        let keys = place(&order, 1, &[new, c]);
        assert_eq!(keys.len(), 2);
        assert!(keys[0].1.as_str() > "V" && keys[1].1 > keys[0].1);
    }
}
