//! Reading what Linux says in `/proc` and `/sys`. The text is read by plain functions,
//! so that they can be tested on any system with text recorded from real ones.

use super::{Counters, CpuTimes, Usage};

/// The processor's times from `/proc/stat`: its first line adds up every core. Waiting
/// for input and output counts as idle; time spent running guests is in `user` already.
pub fn parse_stat(text: &str) -> Option<CpuTimes> {
    let line = text.lines().find(|line| line.starts_with("cpu "))?;
    let fields: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .take(8)
        .map(|field| field.parse().ok())
        .collect::<Option<_>>()?;
    if fields.len() < 5 {
        return None;
    }
    let idle = fields[3] + fields[4];
    let total = fields.iter().sum();
    Some(CpuTimes { idle, total })
}

/// The memory from `/proc/meminfo`: what is used is all of it less what is available,
/// which counts the caches the system can give back.
pub fn parse_meminfo(text: &str) -> Option<Usage> {
    let field = |name: &str| -> Option<u64> {
        let line = text.lines().find(|line| line.starts_with(name))?;
        let kib: u64 = line[name.len()..]
            .trim_start_matches(':')
            .split_whitespace()
            .next()?
            .parse()
            .ok()?;
        Some(kib * 1024)
    };
    let total = field("MemTotal")?;
    let available = field("MemAvailable")?;
    Some(Usage {
        used: total.saturating_sub(available),
        total,
    })
}

/// The bytes received and sent from `/proc/net/dev`, added up over the interfaces
/// `counts` says to count.
pub fn parse_net_dev(text: &str, counts: impl Fn(&str) -> bool) -> Option<Counters> {
    let mut counters = Counters::default();
    for line in text.lines().skip(2) {
        let Some((name, rest)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim();
        if name == "lo" || !counts(name) {
            continue;
        }
        let fields: Vec<u64> = rest
            .split_whitespace()
            .map(|field| field.parse().unwrap_or(0))
            .collect();
        if fields.len() < 9 {
            continue;
        }
        counters.received += fields[0];
        counters.sent += fields[8];
    }
    // A machine with no interface counted has a network that is doing nothing.
    Some(counters)
}

/// A number in a file of `/sys`, such as a card's busyness.
pub fn parse_number(text: &str) -> Option<u64> {
    text.trim().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const STAT: &str = "\
cpu  4705 356 584 3699176 23060 0 277 0 0 0
cpu0 1393280 32966 572056 13343292 6130 0 17875 0 23933 0
intr 114930548 113199788 3 0 5 263 0 4 [... lots more numbers ...]
";

    const MEMINFO: &str = "\
MemTotal:       16314992 kB
MemFree:         6243780 kB
MemAvailable:   12133584 kB
Buffers:          229632 kB
";

    const NET_DEV: &str = "\
Inter-|   Receive                                                |  Transmit
 face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed
    lo: 2776770   11307    0    0    0     0          0         0  2776770   11307    0    0    0     0       0          0
  eth0: 1215645    2751    0    0    0     0          0         0  1782404    4324    0    0    0     427       0          0
docker0:  500       5    0    0    0     0          0         0      600       6    0    0    0     0       0          0
 wlan0: 100       1    0    0    0     0          0         0      200       2    0    0    0     0       0          0
";

    #[test]
    fn the_processor_times_add_up_every_core() {
        let times = parse_stat(STAT).unwrap();
        assert_eq!(times.idle, 3_699_176 + 23_060);
        assert_eq!(times.total, 4705 + 356 + 584 + 3_699_176 + 23_060 + 277);
        assert_eq!(parse_stat("intr 1 2 3"), None);
    }

    #[test]
    fn memory_used_leaves_out_what_the_system_can_give_back() {
        let usage = parse_meminfo(MEMINFO).unwrap();
        assert_eq!(usage.total, 16_314_992 * 1024);
        assert_eq!(usage.used, (16_314_992 - 12_133_584) * 1024);
        assert_eq!(parse_meminfo("MemTotal: 1 kB\n"), None);
    }

    #[test]
    fn the_network_counts_the_interfaces_it_is_told_to() {
        let physical = |name: &str| name.starts_with("eth") || name.starts_with("wlan");
        let counters = parse_net_dev(NET_DEV, physical).unwrap();
        assert_eq!(counters.received, 1_215_645 + 100);
        assert_eq!(counters.sent, 1_782_404 + 200);
        // The loopback never counts.
        let all = parse_net_dev(NET_DEV, |_| true).unwrap();
        assert_eq!(all.received, 1_215_645 + 500 + 100);
    }

    #[test]
    fn a_number_in_sys_is_read_without_its_line_end() {
        assert_eq!(
            parse_number(
                "42
"
            ),
            Some(42)
        );
        assert_eq!(parse_number("busy"), None);
    }
}
