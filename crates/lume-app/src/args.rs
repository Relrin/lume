//! The command line of `lume-gui`.
//!
//! Started with no arguments it opens the terminal. The other ways of starting it are for
//! checks that need a real webview: `--probe` reports what the webview offers, and
//! `--harness` runs a test suite inside it. Both print a report and exit with a status.

use std::path::PathBuf;

/// What the arguments may be, shown when they are not understood.
pub const USAGE: &str = "\
usage: lume-gui [--endpoint <name>]
       lume-gui --probe
       lume-gui --harness <corpus|snapshots|bench> [--report <file>] [--out <dir>]
                [--references <dir>] [--update]

  --endpoint <name>  serve the control endpoint under this name instead of the default
  --probe            report what the webview offers, then exit
  --harness <suite>  run a test suite in the webview, then exit
  --report <file>    also write the suite's report to this file
  --out <dir>        where a suite may write the files it produces
  --references <dir> where the snapshot suite keeps its reference images
  --update           let the snapshot suite replace its reference images

An option's value may follow it, or be joined to it with `=`, as in --endpoint=name.";

/// A test suite the webview can run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Suite {
    /// Replays the recorded terminal sessions and compares the resulting screens.
    Corpus,
    /// Draws the fixed frames on every renderer and compares the pictures.
    Snapshots,
    /// Times drawing and the path bytes take from a session to the screen.
    Bench,
}

impl Suite {
    /// The name the suite goes by on the command line and in the page.
    pub fn name(self) -> &'static str {
        match self {
            Suite::Corpus => "corpus",
            Suite::Snapshots => "snapshots",
            Suite::Bench => "bench",
        }
    }

    fn named(name: &str) -> Option<Self> {
        [Suite::Corpus, Suite::Snapshots, Suite::Bench]
            .into_iter()
            .find(|suite| suite.name() == name)
    }
}

/// What the application was started to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Show a terminal.
    Terminal,
    /// Report on the webview and exit.
    Probe,
    /// Run a suite in the webview and exit.
    Harness(Suite),
}

/// The arguments, taken apart.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Args {
    pub mode: Mode,
    /// The name to serve the control endpoint under, when the default is not wanted.
    pub endpoint: Option<String>,
    /// A file that receives the report as well as standard output. A program without a
    /// console has no standard output anyone can read.
    pub report: Option<PathBuf>,
    /// The directory a suite may write files into.
    pub out: Option<PathBuf>,
    /// The directory the snapshot suite reads its reference images from.
    pub references: Option<PathBuf>,
    /// Whether the snapshot suite may replace its reference images.
    pub update: bool,
}

/// Reads the arguments that follow the program's name.
pub fn parse<I: IntoIterator<Item = String>>(args: I) -> Result<Args, String> {
    let mut parsed = Args {
        mode: Mode::Terminal,
        endpoint: None,
        report: None,
        out: None,
        references: None,
        update: false,
    };
    let mut args = args.into_iter();
    let mut modes = 0;
    while let Some(arg) = args.next() {
        // A value may be joined to its option with `=`. Programs that start the
        // application for a test pass every argument as a switch of its own, so a value
        // that follows as a separate argument does not survive them.
        let (option, joined) = match arg.split_once('=') {
            Some((option, value)) if option.starts_with("--") => (option, Some(value)),
            _ => (arg.as_str(), None),
        };
        let mut took_value = false;
        let mut value = |what: &str| {
            took_value = true;
            match joined {
                Some(value) if !value.is_empty() => Ok(value.to_owned()),
                Some(_) => Err(format!("{option} needs {what}")),
                None => args
                    .next()
                    .filter(|value| !value.starts_with("--"))
                    .ok_or_else(|| format!("{option} needs {what}")),
            }
        };
        match option {
            "--probe" => {
                parsed.mode = Mode::Probe;
                modes += 1;
            }
            "--harness" => {
                let name = value("the name of a suite")?;
                let suite = Suite::named(&name)
                    .ok_or_else(|| format!("there is no suite called {name}"))?;
                parsed.mode = Mode::Harness(suite);
                modes += 1;
            }
            "--endpoint" => parsed.endpoint = Some(value("a name")?),
            "--report" => parsed.report = Some(PathBuf::from(value("a file")?)),
            "--out" => parsed.out = Some(PathBuf::from(value("a directory")?)),
            "--references" => parsed.references = Some(PathBuf::from(value("a directory")?)),
            "--update" => parsed.update = true,
            // What the system adds when it starts an application from its launcher.
            other if other.starts_with("-psn_") => {}
            other => return Err(format!("{other} is not an argument lume-gui knows")),
        }
        if joined.is_some() && !took_value {
            return Err(format!("{option} takes no value"));
        }
    }
    if modes > 1 {
        return Err("--probe and --harness cannot be used together".to_owned());
    }
    let harness = matches!(parsed.mode, Mode::Harness(_));
    let for_harness = parsed.report.is_some()
        || parsed.out.is_some()
        || parsed.references.is_some()
        || parsed.update;
    if !harness && for_harness {
        return Err("--report, --out, --references and --update belong to --harness".to_owned());
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(args: &[&str]) -> Result<Args, String> {
        parse(args.iter().map(|arg| (*arg).to_owned()))
    }

    #[test]
    fn no_arguments_start_the_terminal() {
        let args = parsed(&[]).unwrap();
        assert_eq!(args.mode, Mode::Terminal);
        assert_eq!(args.endpoint, None);
    }

    #[test]
    fn the_endpoint_can_be_named() {
        let args = parsed(&["--endpoint", "trial-7"]).unwrap();
        assert_eq!(args.endpoint.as_deref(), Some("trial-7"));
        assert_eq!(args.mode, Mode::Terminal);
    }

    #[test]
    fn a_harness_run_takes_its_suite_and_where_to_write() {
        let args = parsed(&[
            "--harness",
            "snapshots",
            "--report",
            "r.json",
            "--out",
            "o",
            "--references",
            "refs",
            "--update",
        ])
        .unwrap();
        assert_eq!(args.mode, Mode::Harness(Suite::Snapshots));
        assert_eq!(args.report, Some(PathBuf::from("r.json")));
        assert_eq!(args.out, Some(PathBuf::from("o")));
        assert_eq!(args.references, Some(PathBuf::from("refs")));
        assert!(args.update);
    }

    #[test]
    fn a_value_may_be_joined_to_its_option() {
        let args = parsed(&["--endpoint=trial-7"]).unwrap();
        assert_eq!(args.endpoint.as_deref(), Some("trial-7"));

        let args = parsed(&["--harness=corpus", "--report=a=b.json", "--out", "o"]).unwrap();
        assert_eq!(args.mode, Mode::Harness(Suite::Corpus));
        // Only the first `=` joins; the rest is the value's own.
        assert_eq!(args.report, Some(PathBuf::from("a=b.json")));
        assert_eq!(args.out, Some(PathBuf::from("o")));

        assert!(parsed(&["--endpoint="]).is_err());
        assert!(parsed(&["--probe=yes"]).is_err());
        assert!(parsed(&["--harness", "corpus", "--update=1"]).is_err());
        assert!(parsed(&["--nonesuch=1"]).is_err());
    }

    #[test]
    fn every_suite_is_found_by_its_name() {
        for suite in [Suite::Corpus, Suite::Snapshots, Suite::Bench] {
            let args = parsed(&["--harness", suite.name()]).unwrap();
            assert_eq!(args.mode, Mode::Harness(suite));
        }
    }

    #[test]
    fn what_is_not_understood_is_refused() {
        assert!(parsed(&["--verbose"]).is_err());
        assert!(parsed(&["--harness"]).is_err());
        assert!(parsed(&["--harness", "everything"]).is_err());
        assert!(parsed(&["--endpoint", "--probe"]).is_err());
        assert!(parsed(&["--probe", "--harness", "corpus"]).is_err());
        assert!(parsed(&["--report", "r.json"]).is_err());
        assert!(parsed(&["--probe", "--update"]).is_err());
    }

    #[test]
    fn a_launcher_serial_number_is_ignored() {
        assert_eq!(parsed(&["-psn_0_12345"]).unwrap().mode, Mode::Terminal);
    }
}
