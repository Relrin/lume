//! Snapshots that are not as the library saved them.
//!
//! A snapshot may come from another machine, so it may hold anything. Loading one that is
//! damaged, or made to do harm, has to end in a terminal or in an error, and never in the
//! end of the process. So the loading is done in a process of its own, which says what it
//! is about to load before it does: when it ends any other way, that is what did it.

use std::io::Write as _;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use lume_vt::{Limits, Terminal};

/// Set in the process that loads, to the seeds it goes through: `first..end`.
const SEEDS: &str = "LUME_DAMAGED_SEEDS";
/// What a snapshot may take to be loaded.
const LIMITS: Limits = Limits {
    cells: 512 * 1024,
    bytes: 256 * 1024 * 1024,
};

/// Beyond this a loading process is taken to hang.
const LIMIT: Duration = Duration::from_secs(120);

/// Numbers from a seed, the same every time.
struct Numbers(u64);

impl Numbers {
    fn next(&mut self) -> u64 {
        // SplitMix64.
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn below(&mut self, end: usize) -> usize {
        usize::try_from(self.next() % end.max(1) as u64).unwrap()
    }
}

/// Sound snapshots to damage: of a screen with a bit of everything, with scrollback, on
/// the alternate screen, and of an empty terminal.
fn sound() -> Vec<Vec<u8>> {
    let mut everything = Terminal::new(40, 10).unwrap();
    everything.write(
        "\x1b[1;31mred\x1b[0m \x1b[4:3munder\x1b[0m \x1b[38;2;1;2;3mrgb\x1b[0m\r\n\
         \x1b]8;id=x;https://example.test/\x1b\\link\x1b]8;;\x1b\\ 日本 e\u{301} 👩\u{200d}💻\r\n\
         \x1b[?2004h\x1b[2;8r\x1b]2;title\x1b\\\x1b[>1u\x1b(0lqk\x1b(B\x1b[3;3Hhere\
         \x1b_Ga=T,f=24,s=2,v=2;////////////////\x1b\\\x1b["
            .as_bytes(),
    );
    let mut scrolled = Terminal::new(20, 5).unwrap();
    for line in 0..200 {
        scrolled.write(format!("\x1b[3{}mline {line}\r\n", line % 8).as_bytes());
    }
    let mut alternate = Terminal::new(30, 8).unwrap();
    alternate.write(b"below\x1b[?1049h\x1b[H\x1b[44mabove");
    [
        everything,
        scrolled,
        alternate,
        Terminal::new(80, 24).unwrap(),
    ]
    .iter()
    .map(|terminal| terminal.snapshot().unwrap())
    .collect()
}

/// How a snapshot begins, before its first record.
const ENVELOPE: usize = 10;
/// How a record begins: its tag, the length of what it holds, and a checksum of both.
const HEADER: usize = 10;

/// The checksum of a record: CRC32C, as the snapshot format has it.
fn checksum(parts: [&[u8]; 2]) -> u32 {
    let mut crc = u32::MAX;
    for byte in parts.into_iter().flatten() {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0x82f6_3b78 & (crc & 1).wrapping_neg());
        }
    }
    !crc
}

/// The records of a sound snapshot: the tag of each, and what it holds.
fn records(snapshot: &[u8]) -> Vec<([u8; 2], Vec<u8>)> {
    let mut found = Vec::new();
    let mut at = ENVELOPE;
    while at < snapshot.len() {
        let head = &snapshot[at..at + HEADER];
        let length = u32::from_le_bytes(head[2..6].try_into().unwrap()) as usize;
        let body = &snapshot[at + HEADER..at + HEADER + length];
        assert_eq!(
            checksum([&head[..6], body]),
            u32::from_le_bytes(head[6..10].try_into().unwrap()),
            "the checksum is the one the library makes"
        );
        found.push(([head[0], head[1]], body.to_vec()));
        at += HEADER + length;
    }
    found
}

/// A snapshot of these records, with the lengths and checksums they ought to have.
fn sealed(envelope: &[u8], records: &[([u8; 2], Vec<u8>)]) -> Vec<u8> {
    let mut out = envelope.to_vec();
    for (tag, body) in records {
        let mut head = tag.to_vec();
        head.extend(u32::try_from(body.len()).unwrap().to_le_bytes());
        let sum = checksum([&head, body]);
        out.extend(head);
        out.extend(sum.to_le_bytes());
        out.extend(body);
    }
    out
}

/// Changes some bytes in one of a few ways.
fn change(bytes: &mut Vec<u8>, numbers: &mut Numbers) {
    if bytes.is_empty() {
        bytes.extend((0..numbers.below(16)).map(|_| numbers.next() as u8));
        return;
    }
    // Most of what matters is at the head.
    let at = if numbers.below(3) == 0 {
        numbers.below(bytes.len().min(96))
    } else {
        numbers.below(bytes.len())
    };
    match numbers.below(9) {
        0 | 1 => bytes[at] ^= 1 << numbers.below(8),
        2 => bytes[at] = numbers.next() as u8,
        3 => bytes.truncate(at),
        4 | 5 => {
            // A number far too large, or nothing at all.
            let end = (at + [1, 2, 4, 8][numbers.below(4)]).min(bytes.len());
            bytes[at..end].fill([0xff, 0x00, 0x7f][numbers.below(3)]);
        }
        6 => {
            let end = (at + 1 + numbers.below(32)).min(bytes.len());
            bytes.drain(at..end);
        }
        7 => {
            let end = (at + 1 + numbers.below(32)).min(bytes.len());
            let again = bytes[at..end].to_vec();
            bytes.splice(at..at, again);
        }
        _ => {
            let more: Vec<u8> = (0..numbers.below(16))
                .map(|_| numbers.next() as u8)
                .collect();
            bytes.splice(at..at, more);
        }
    }
}

/// A snapshot that somebody meant to do harm with, or that was cut or mixed up on its
/// way. What is in its records is changed and the records are sealed again: without that
/// the checksums would refuse nearly all of them before anything is read.
fn damage(sound: &[Vec<u8>], numbers: &mut Numbers) -> Vec<u8> {
    let of = &sound[numbers.below(sound.len())];
    let mut records = records(of);
    for _ in 0..=numbers.below(3) {
        let nth = numbers.below(records.len());
        match numbers.below(16) {
            0 => {
                records.remove(nth);
            }
            1 => {
                let other = numbers.below(records.len());
                records.swap(nth, other);
            }
            2 => {
                let again = records[nth].clone();
                records.insert(nth, again);
            }
            3 => records[nth].0 = [numbers.below(14) as u8, 0],
            _ => change(&mut records[nth].1, numbers),
        }
        if records.is_empty() {
            break;
        }
    }
    let mut bytes = sealed(&of[..ENVELOPE], &records);
    // And now and then one that is not sealed as it ought to be.
    if numbers.below(20) == 0 {
        change(&mut bytes, numbers);
    }
    bytes
}

/// Loads the snapshot of a seed, and uses what came of it as a view would. Says whether
/// anything came of it.
fn load(sound: &[Vec<u8>], seed: u64) -> bool {
    let mut numbers = Numbers(seed);
    let bytes = damage(sound, &mut numbers);
    let Ok((mut terminal, length)) = Terminal::from_foreign_snapshot(&bytes, LIMITS) else {
        return false;
    };
    assert!(length <= bytes.len());
    let _ = terminal.screen_text();
    terminal.write("more \x1b[H日本\x1b[2J\x1b[?1049h\x1b[?1049l\r\nend\x1bc".as_bytes());
    let _ = terminal.resize(31, 9);
    let _ = terminal.resize(132, 50);
    let _ = terminal.screen_text();
    let _ = terminal.snapshot();
    true
}

/// Not a test of its own: what the test below runs in another process.
#[test]
#[ignore = "run by the test that starts a process for it"]
fn loading() {
    let Ok(seeds) = std::env::var(SEEDS) else {
        return;
    };
    let (first, end) = seeds.split_once("..").unwrap();
    let sound = sound();
    let (mut loaded, mut count) = (0, 0);
    for seed in first.parse().unwrap()..end.parse().unwrap() {
        println!("seed {seed}");
        std::io::stdout().flush().unwrap();
        loaded += usize::from(load(&sound, seed));
        count += 1;
    }
    println!("loaded {loaded} of {count}");
    println!("done");
}

#[test]
fn a_damaged_snapshot_is_loaded_or_refused_and_ends_nothing() {
    let seeds = std::env::var(SEEDS).unwrap_or_else(|_| "0..3000".to_owned());
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["loading", "--exact", "--ignored", "--nocapture"])
        .env(SEEDS, &seeds)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let output = child.stdout.take().unwrap();
    let reading = std::thread::spawn(move || std::io::read_to_string(output).unwrap());
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status.to_string();
        }
        if started.elapsed() > LIMIT {
            child.kill().unwrap();
            child.wait().unwrap();
            break "no end".to_owned();
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let said = reading.join().unwrap();
    let last = said.lines().rfind(|line| line.starts_with("seed"));
    assert!(
        said.lines().any(|line| line == "done"),
        "loading ended with {status} at {}",
        last.unwrap_or("its beginning")
    );
}
