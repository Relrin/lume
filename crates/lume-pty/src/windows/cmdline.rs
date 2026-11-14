//! Finding the program to start and building the command line it receives.
//!
//! Windows hands a process one command-line string, which the program splits into
//! arguments itself. Nearly every program splits it by the C runtime's rules, so the
//! arguments are quoted by those rules here. The executable is found by this module and
//! passed separately as a full path: left to itself, process creation searches the
//! current directory before `PATH`, which would run whatever file of the right name
//! happens to be in the working directory.

use std::ffi::{OsStr, OsString};
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};

/// The extensions tried for a name given without one. Only these can be started
/// directly; scripts need their interpreter named as the program.
const EXECUTABLE_EXTENSIONS: [&str; 2] = ["exe", "com"];

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, what.into())
}

/// A string as NUL-terminated UTF-16, refusing one that contains a NUL.
pub(crate) fn wide(text: &OsStr) -> io::Result<Vec<u16>> {
    let mut wide: Vec<u16> = text.encode_wide().collect();
    if wide.contains(&0) {
        return Err(invalid("a string passed to the system contains a NUL"));
    }
    wide.push(0);
    Ok(wide)
}

/// Rewrites a path in the verbatim form (`\\?\C:\dir`, `\\?\UNC\server\share`) to the
/// ordinary form (`C:\dir`, `\\server\share`) where one exists.
///
/// Canonicalising a path on Windows produces the verbatim form, and many programs do not
/// accept it: `cmd.exe` given it as a working directory reports an unsupported UNC path
/// and starts in the Windows directory instead.
pub(crate) fn plain_path(path: &Path) -> PathBuf {
    use std::path::{Component, Prefix};

    let mut components = path.components();
    let Some(Component::Prefix(prefix)) = components.next() else {
        return path.to_path_buf();
    };
    let mut plain = match prefix.kind() {
        Prefix::VerbatimDisk(letter) => OsString::from(format!("{}:", char::from(letter))),
        Prefix::VerbatimUNC(server, share) => {
            let mut unc = OsString::from(r"\\");
            unc.push(server);
            unc.push(r"\");
            unc.push(share);
            unc
        }
        _ => return path.to_path_buf(),
    };
    plain.push(components.as_path().as_os_str());
    PathBuf::from(plain)
}

/// Finds the executable for `program` and returns its absolute path.
///
/// A program given with a directory part is taken relative to `cwd`. A bare name is
/// looked up in the absolute directories of `path`, in order, and nowhere else.
pub(crate) fn resolve_program(
    program: &OsStr,
    cwd: &Path,
    path: Option<&OsStr>,
) -> io::Result<PathBuf> {
    if program.is_empty() {
        return Err(invalid("the program name is empty"));
    }
    let given = Path::new(program);
    let not_found = || {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("the program `{}` was not found", given.display()),
        )
    };

    let has_directory = given.components().count() > 1 || given.has_root();
    let found = if has_directory {
        executable_at(&cwd.join(given))
    } else {
        path.into_iter()
            .flat_map(std::env::split_paths)
            .filter(|dir| dir.is_absolute())
            .find_map(|dir| executable_at(&dir.join(given)))
    }
    .ok_or_else(not_found)?;

    let extension = found
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase());
    if matches!(extension.as_deref(), Some("bat" | "cmd")) {
        return Err(invalid(format!(
            "`{}` is a batch file; start it through cmd.exe",
            found.display()
        )));
    }
    std::path::absolute(found)
}

/// The file at `base`, or at `base` with an executable extension added.
fn executable_at(base: &Path) -> Option<PathBuf> {
    let with_extension = EXECUTABLE_EXTENSIONS.iter().map(|extension| {
        let mut name = base.as_os_str().to_owned();
        name.push(".");
        name.push(extension);
        PathBuf::from(name)
    });
    // A name that already has an extension is tried as given first, so `tool.exe` never
    // becomes `tool.exe.exe`; a name without one prefers the executable over a file that
    // merely shares its stem.
    let candidates: Vec<PathBuf> = if base.extension().is_some() {
        std::iter::once(base.to_path_buf())
            .chain(with_extension)
            .collect()
    } else {
        with_extension
            .chain(std::iter::once(base.to_path_buf()))
            .collect()
    };
    candidates.into_iter().find(|candidate| candidate.is_file())
}

/// Builds the command line for `program` and `args` as NUL-terminated UTF-16.
pub(crate) fn command_line(program: &OsStr, args: &[OsString]) -> io::Result<Vec<u16>> {
    let quote = u16::from(b'"');
    let mut line: Vec<u16> = Vec::new();

    // The program name is not unescaped like the other arguments: it ends at the first
    // space, or at the closing quote if it starts with one. A quote inside it therefore
    // cannot be represented.
    let name: Vec<u16> = program.encode_wide().collect();
    if name.is_empty() || name.contains(&quote) {
        return Err(invalid("the program name is empty or contains a quote"));
    }
    line.push(quote);
    line.extend_from_slice(&name);
    line.push(quote);

    for arg in args {
        line.push(u16::from(b' '));
        append_argument(&mut line, &arg.encode_wide().collect::<Vec<u16>>());
    }
    if line.contains(&0) {
        return Err(invalid("an argument contains a NUL"));
    }
    line.push(0);
    Ok(line)
}

/// Appends one argument so that the C runtime's splitting yields it back unchanged.
fn append_argument(line: &mut Vec<u16>, arg: &[u16]) {
    let quote = u16::from(b'"');
    let backslash = u16::from(b'\\');
    let needs_quotes = arg.is_empty()
        || arg
            .iter()
            .any(|&unit| unit == quote || matches!(unit, 0x20 | 0x09 | 0x0a | 0x0b));
    if !needs_quotes {
        line.extend_from_slice(arg);
        return;
    }

    line.push(quote);
    let mut backslashes = 0;
    for &unit in arg {
        if unit == backslash {
            backslashes += 1;
            continue;
        }
        // Backslashes are literal unless a quote follows them. Before a quote each one
        // must be doubled, and the quote itself escaped with one more.
        let escapes = if unit == quote {
            backslashes * 2 + 1
        } else {
            backslashes
        };
        line.extend(std::iter::repeat_n(backslash, escapes));
        line.push(unit);
        backslashes = 0;
    }
    // Trailing backslashes precede the closing quote, so they are doubled too.
    line.extend(std::iter::repeat_n(backslash, backslashes * 2));
    line.push(quote);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(program: &str, args: &[&str]) -> String {
        let args: Vec<OsString> = args.iter().map(OsString::from).collect();
        let wide = command_line(OsStr::new(program), &args).unwrap();
        assert_eq!(wide.last(), Some(&0));
        String::from_utf16(&wide[..wide.len() - 1]).unwrap()
    }

    #[test]
    fn arguments_are_quoted_by_the_c_runtime_rules() {
        let cases: [(&[&str], &str); 9] = [
            (&[], r#""app""#),
            (&["plain"], r#""app" plain"#),
            (&["two words"], r#""app" "two words""#),
            (&[""], r#""app" """#),
            (&[r#"say "hi""#], r#""app" "say \"hi\"""#),
            (&[r"C:\dir\"], r#""app" C:\dir\"#),
            (&[r"C:\my dir\"], r#""app" "C:\my dir\\""#),
            (&[r#"a\\"b"#], r#""app" "a\\\\\"b""#),
            (&["a", "b c", "d"], r#""app" a "b c" d"#),
        ];
        for (args, expected) in cases {
            assert_eq!(line("app", args), expected, "{args:?}");
        }
    }

    #[test]
    fn a_program_name_with_a_quote_or_a_nul_is_refused() {
        assert!(command_line(OsStr::new("a\"b"), &[]).is_err());
        assert!(command_line(OsStr::new(""), &[]).is_err());
        assert!(command_line(OsStr::new("app"), &[OsString::from("a\0b")]).is_err());
        assert!(wide(OsStr::new("a\0b")).is_err());
    }

    fn system32() -> PathBuf {
        PathBuf::from(std::env::var_os("SystemRoot").expect("SystemRoot is set")).join("System32")
    }

    #[test]
    fn a_bare_name_is_found_on_the_path_with_or_without_its_extension() {
        let dir = system32();
        let path = std::env::join_paths([Path::new(r"C:\no\such\dir"), &dir]).unwrap();
        let cwd = std::env::temp_dir();
        for name in ["cmd", "cmd.exe", "CMD.EXE"] {
            let found = resolve_program(OsStr::new(name), &cwd, Some(&path)).unwrap();
            assert!(found.is_absolute());
            assert_eq!(
                found.to_string_lossy().to_lowercase(),
                dir.join("cmd.exe").to_string_lossy().to_lowercase()
            );
        }
    }

    #[test]
    fn a_bare_name_is_never_looked_up_in_the_working_directory() {
        // The working directory holds cmd.exe, but neither it nor a relative `PATH`
        // entry naming it is searched.
        let cwd = system32();
        let error = resolve_program(OsStr::new("cmd.exe"), &cwd, None).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        let error =
            resolve_program(OsStr::new("cmd.exe"), &cwd, Some(OsStr::new("."))).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn a_name_with_a_directory_part_is_taken_relative_to_the_working_directory() {
        let found = resolve_program(OsStr::new(r".\cmd"), &system32(), None).unwrap();
        assert!(found.is_absolute());
        assert!(
            found
                .to_string_lossy()
                .to_lowercase()
                .ends_with(r"system32\cmd.exe")
        );

        let absolute = system32().join("cmd.exe");
        let found = resolve_program(absolute.as_os_str(), &std::env::temp_dir(), None).unwrap();
        assert_eq!(found, absolute);
    }

    #[test]
    fn verbatim_paths_are_rewritten_to_the_ordinary_form() {
        let cases = [
            (r"\\?\C:\Users\someone", r"C:\Users\someone"),
            (r"\\?\C:\", r"C:\"),
            (r"\\?\UNC\server\share\dir", r"\\server\share\dir"),
            (r"C:\Users\someone", r"C:\Users\someone"),
            (r"\\server\share\dir", r"\\server\share\dir"),
            (r"relative\dir", r"relative\dir"),
            // A verbatim path with no ordinary equivalent is left alone.
            (
                r"\\?\Volume{b75e2c83-0000-0000-0000-602f00000000}\dir",
                r"\\?\Volume{b75e2c83-0000-0000-0000-602f00000000}\dir",
            ),
        ];
        for (given, expected) in cases {
            assert_eq!(plain_path(Path::new(given)), Path::new(expected), "{given}");
        }
    }

    #[test]
    fn a_missing_program_is_not_found() {
        let path = system32();
        let error = resolve_program(
            OsStr::new("no-such-program-here"),
            &std::env::temp_dir(),
            Some(path.as_os_str()),
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
    }
}
