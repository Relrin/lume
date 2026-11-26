//! What the service tests share: folders laid out as Lume's are, programs to run, and
//! small servers on this computer to fetch from.

#![allow(dead_code, reason = "each test file uses part of this")]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use lume_services::Places;
use lume_services::net::Lookup;

static NEXT: AtomicUsize = AtomicUsize::new(0);

/// A folder of the test's own, removed when it is dropped.
pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    pub fn new(what: &str) -> Self {
        let number = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "lume-services-it-{}-{number}-{what}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("a temporary folder can be made");
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Writes a file under the folder, making the folders on the way.
    pub fn write(&self, relative: &str, bytes: &[u8]) -> PathBuf {
        let path = self.path.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("the folders can be made");
        }
        std::fs::write(&path, bytes).expect("the file can be written");
        path
    }

    /// Makes a folder under this one.
    pub fn folder(&self, relative: &str) -> PathBuf {
        let path = self.path.join(relative);
        std::fs::create_dir_all(&path).expect("the folder can be made");
        path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        // Junctions first, so removing the folder does not reach through one.
        remove_links(&self.path);
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn remove_links(folder: &Path) {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() || is_junction(&metadata) => {
                let _ = std::fs::remove_dir(&path).or_else(|_| std::fs::remove_file(&path));
            }
            Ok(metadata) if metadata.is_dir() => remove_links(&path),
            _ => {}
        }
    }
}

#[cfg(windows)]
fn is_junction(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    // A reparse point that is a folder: a junction or a folder link.
    metadata.file_attributes() & 0x400 != 0
}

#[cfg(not(windows))]
fn is_junction(_metadata: &std::fs::Metadata) -> bool {
    false
}

/// A layout as Lume's: a workspace, a home, and Lume's own folders (configuration, state
/// with the plugin's data folder in it, the install folder with Lume's program), and a
/// folder outside all of them.
pub struct Layout {
    pub dir: TempDir,
    pub places: Places,
}

impl Layout {
    pub fn new(what: &str) -> Self {
        let dir = TempDir::new(what);
        for folder in [
            "ws",
            "home",
            "config",
            "state/plugin-data/a.x",
            "install",
            "outside",
        ] {
            dir.folder(folder);
        }
        let lume = dir.write(&format!("install/{}", exe("lume")), b"");
        // A program, as the installed one is: a search along the PATH passes over a file
        // that cannot be run.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&lume, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let places = Places {
            workspace: Some(dir.path().join("ws")),
            home: dir.path().join("home"),
            plugin_data: dir.path().join("state").join("plugin-data").join("a.x"),
            forbidden: vec![
                dir.path().join("config"),
                dir.path().join("state"),
                dir.path().join("install"),
            ],
            own_programs: vec![lume],
        };
        Self { dir, places }
    }

    pub fn path(&self, relative: &str) -> PathBuf {
        self.dir.path().join(relative)
    }

    /// An absolute path under the layout, written with forward slashes, as a plugin
    /// would ask for it.
    pub fn asked(&self, relative: &str) -> String {
        let root = self.dir.path().display().to_string().replace('\\', "/");
        format!("{root}/{relative}")
    }
}

/// A program's file name on this system.
pub fn exe(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_owned()
    }
}

pub fn grants(list: &[&str]) -> Vec<String> {
    list.iter().map(|grant| (*grant).to_owned()).collect()
}

/// Makes a link to a folder, or says why it cannot be made here. On Windows that needs
/// a privilege most users do not have, unless developer mode is on.
pub fn folder_link(target: &Path, link: &Path) -> Result<(), String> {
    #[cfg(windows)]
    let made = std::os::windows::fs::symlink_dir(target, link);
    #[cfg(unix)]
    let made = std::os::unix::fs::symlink(target, link);
    made.map_err(|error| format!("a link to a folder could not be made here ({error})"))
}

/// Makes a link to a file, or says why it cannot be made here.
pub fn file_link(target: &Path, link: &Path) -> Result<(), String> {
    #[cfg(windows)]
    let made = std::os::windows::fs::symlink_file(target, link);
    #[cfg(unix)]
    let made = std::os::unix::fs::symlink(target, link);
    made.map_err(|error| format!("a link to a file could not be made here ({error})"))
}

/// Makes a junction on Windows, which needs no privilege.
#[cfg(windows)]
pub fn junction(target: &Path, link: &Path) {
    // `mklink` would take a forward slash for the start of a switch.
    let native = |path: &Path| path.display().to_string().replace('/', "\\");
    let status = std::process::Command::new("cmd")
        .args(["/d", "/c", "mklink", "/J"])
        .arg(native(link))
        .arg(native(target))
        .stdout(std::process::Stdio::null())
        .status()
        .expect("cmd runs");
    assert!(status.success(), "the junction could be made");
}

/// The folders this system's own programs are in.
pub fn system_path() -> std::ffi::OsString {
    if cfg!(windows) {
        let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
        std::env::join_paths([PathBuf::from(root).join("System32")]).unwrap()
    } else {
        "/bin:/usr/bin".into()
    }
}

/// A request a small server got.
#[derive(Clone, Debug, Default)]
pub struct Got {
    pub head: String,
    pub body: Vec<u8>,
}

impl Got {
    pub fn header(&self, name: &str) -> Option<String> {
        self.head.lines().skip(1).find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.trim()
                .eq_ignore_ascii_case(name)
                .then(|| value.trim().to_owned())
        })
    }
}

/// A small HTTP server on this computer that answers each connection with the next of
/// its answers, then stops. It stops by itself after ten seconds if not every answer was
/// asked for.
pub struct Server {
    pub port: u16,
    got: Arc<Mutex<Vec<Got>>>,
}

impl Server {
    pub fn new(answers: Vec<Vec<u8>>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a port on this computer");
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();
        let got = Arc::new(Mutex::new(Vec::new()));
        let record = Arc::clone(&got);
        std::thread::spawn(move || {
            let until = Instant::now() + Duration::from_secs(10);
            for answer in answers {
                let stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break Some(stream),
                        Err(_) if Instant::now() < until => {
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(_) => break None,
                    }
                };
                let Some(mut stream) = stream else {
                    return;
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let request = read_request(&mut stream);
                record.lock().unwrap().push(request);
                let _ = stream.write_all(&answer);
                let _ = stream.flush();
            }
        });
        Self { port, got }
    }

    /// The requests it got so far.
    pub fn got(&self) -> Vec<Got> {
        self.got.lock().unwrap().clone()
    }

    pub fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }
}

fn read_request(stream: &mut std::net::TcpStream) -> Got {
    let mut reader = BufReader::new(stream);
    let mut head = String::new();
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        if line == "\r\n" {
            break;
        }
        head.push_str(&line);
    }
    let mut got = Got {
        head,
        body: Vec::new(),
    };
    if let Some(length) = got
        .header("content-length")
        .and_then(|length| length.parse().ok())
    {
        let mut body = vec![0u8; length];
        if reader.read_exact(&mut body).is_ok() {
            got.body = body;
        }
    }
    got
}

/// An answer with a status, extra header lines and a body.
pub fn answer(status: &str, headers: &[&str], body: &[u8]) -> Vec<u8> {
    let mut text = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    for header in headers {
        text.push_str(header);
        text.push_str("\r\n");
    }
    text.push_str("\r\n");
    let mut bytes = text.into_bytes();
    bytes.extend_from_slice(body);
    bytes
}

/// A redirect to `location`.
pub fn redirect(status: &str, location: &str) -> Vec<u8> {
    answer(status, &[&format!("Location: {location}")], b"")
}

/// A lookup that knows only made-up names, so no test asks the real DNS: each name
/// leads to the addresses given, with the port asked for.
pub fn lookup(names: &[(&str, &str)]) -> Arc<Lookup> {
    let names: Vec<(String, std::net::IpAddr)> = names
        .iter()
        .map(|(name, address)| ((*name).to_owned(), address.parse().unwrap()))
        .collect();
    Arc::new(move |host: &str, port: u16| {
        let found: Vec<SocketAddr> = names
            .iter()
            .filter(|(name, _)| name == host)
            .map(|(_, address)| SocketAddr::new(*address, port))
            .collect();
        if found.is_empty() {
            Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "no such name",
            ))
        } else {
            Ok(found)
        }
    })
}

/// A GET request with room for a small body.
pub fn get(url: &str) -> lume_services::net::FetchRequest {
    lume_services::net::FetchRequest {
        method: "GET".to_owned(),
        url: url.to_owned(),
        headers: Vec::new(),
        body: None,
        timeout: Duration::from_secs(10),
        max_bytes: 1024 * 1024,
    }
}
