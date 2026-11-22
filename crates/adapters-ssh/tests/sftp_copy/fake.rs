//! The fake host: an SFTP server that keeps its files in memory and answers
//! over an in-memory pipe.

use russh_sftp::client::SftpSession;
use russh_sftp::protocol::{
    Attrs, Data, File, FileAttributes, Handle, Name, OpenFlags, Status, StatusCode, Version,
};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex};

pub const HOME: &str = "/home/me";

#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Dir,
    File(Vec<u8>),
    Link,
}

/// The files of one fake host, by absolute path.
#[derive(Default)]
pub struct Disk {
    pub nodes: BTreeMap<String, Node>,
    /// Names a hostile server lists in a folder although no honest one
    /// could hold them: (folder, name).
    pub rogue: Vec<(String, String)>,
    /// Writes fail once a file is this long.
    pub full_at: Option<usize>,
}

pub type Shared = Arc<Mutex<Disk>>;

pub fn disk(nodes: &[(&str, Node)]) -> Shared {
    let mut disk = Disk::default();
    for path in ["/", "/home", HOME, "/tmp"] {
        disk.nodes.insert(path.into(), Node::Dir);
    }
    for (path, node) in nodes {
        disk.nodes.insert((*path).into(), node.clone());
    }
    Arc::new(Mutex::new(disk))
}

pub fn file(bytes: &[u8]) -> Node {
    Node::File(bytes.to_vec())
}

/// Every path of a disk below `root`, for comparing trees.
pub fn paths(disk: &Shared, root: &str) -> Vec<String> {
    let prefix = format!("{root}/");
    disk.lock()
        .unwrap()
        .nodes
        .keys()
        .filter(|path| path.starts_with(&prefix))
        .cloned()
        .collect()
}

pub fn content(disk: &Shared, path: &str) -> Option<Vec<u8>> {
    match disk.lock().unwrap().nodes.get(path) {
        Some(Node::File(bytes)) => Some(bytes.clone()),
        _ => None,
    }
}

pub fn absolute(path: &str) -> String {
    let joined = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("{HOME}/{path}")
    };
    let mut parts: Vec<&str> = Vec::new();
    for part in joined.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            part => parts.push(part),
        }
    }
    format!("/{}", parts.join("/"))
}

pub fn attrs(node: &Node) -> FileAttributes {
    let (mode, size) = match node {
        Node::Dir => (0o040755, 0),
        Node::File(bytes) => (0o100640, bytes.len() as u64),
        Node::Link => (0o120777, 0),
    };
    FileAttributes {
        size: Some(size),
        permissions: Some(mode),
        mtime: Some(1_700_000_000),
        ..FileAttributes::empty()
    }
}

pub fn ok(id: u32) -> Status {
    Status {
        id,
        status_code: StatusCode::Ok,
        error_message: "Ok".into(),
        language_tag: "en-US".into(),
    }
}

pub struct Fake {
    disk: Shared,
    /// Open handle to the path it names.
    handles: HashMap<String, String>,
    /// Folder handles whose listing was already sent.
    listed: HashSet<String>,
    next: u32,
}

impl Fake {
    fn handle(&mut self, id: u32, path: String) -> Handle {
        self.next += 1;
        let handle = format!("h{}", self.next);
        self.handles.insert(handle.clone(), path);
        Handle { id, handle }
    }

    fn stat_path(&self, id: u32, path: &str) -> Result<Attrs, StatusCode> {
        let disk = self.disk.lock().unwrap();
        let node = disk
            .nodes
            .get(&absolute(path))
            .ok_or(StatusCode::NoSuchFile)?;
        Ok(Attrs {
            id,
            attrs: attrs(node),
        })
    }
}

impl russh_sftp::server::Handler for Fake {
    type Error = StatusCode;

    fn unimplemented(&self) -> Self::Error {
        StatusCode::OpUnsupported
    }

    async fn init(
        &mut self,
        _version: u32,
        _extensions: HashMap<String, String>,
    ) -> Result<Version, Self::Error> {
        Ok(Version::new())
    }

    async fn open(
        &mut self,
        id: u32,
        filename: String,
        pflags: OpenFlags,
        _attrs: FileAttributes,
    ) -> Result<Handle, Self::Error> {
        let path = absolute(&filename);
        {
            let mut disk = self.disk.lock().unwrap();
            match disk.nodes.get(&path) {
                Some(Node::File(_)) if pflags.contains(OpenFlags::TRUNCATE) => {
                    disk.nodes.insert(path.clone(), Node::File(Vec::new()));
                }
                Some(Node::File(_)) => {}
                Some(_) => return Err(StatusCode::Failure),
                None if pflags.contains(OpenFlags::CREATE) => {
                    let parent = path.rsplit_once('/').map(|(dir, _)| dir).unwrap_or("");
                    let parent = if parent.is_empty() { "/" } else { parent };
                    if disk.nodes.get(parent) != Some(&Node::Dir) {
                        return Err(StatusCode::NoSuchFile);
                    }
                    disk.nodes.insert(path.clone(), Node::File(Vec::new()));
                }
                None => return Err(StatusCode::NoSuchFile),
            }
        }
        Ok(self.handle(id, path))
    }

    async fn close(&mut self, id: u32, handle: String) -> Result<Status, Self::Error> {
        self.handles.remove(&handle);
        self.listed.remove(&handle);
        Ok(ok(id))
    }

    async fn read(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        len: u32,
    ) -> Result<Data, Self::Error> {
        let path = self.handles.get(&handle).ok_or(StatusCode::Failure)?;
        let disk = self.disk.lock().unwrap();
        let Some(Node::File(bytes)) = disk.nodes.get(path) else {
            return Err(StatusCode::NoSuchFile);
        };
        let start = offset as usize;
        if start >= bytes.len() {
            return Err(StatusCode::Eof);
        }
        let end = (start + len as usize).min(bytes.len());
        Ok(Data {
            id,
            data: bytes[start..end].to_vec(),
        })
    }

    async fn write(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        data: Vec<u8>,
    ) -> Result<Status, Self::Error> {
        let path = self.handles.get(&handle).ok_or(StatusCode::Failure)?;
        let mut disk = self.disk.lock().unwrap();
        let full_at = disk.full_at;
        let Some(Node::File(bytes)) = disk.nodes.get_mut(path) else {
            return Err(StatusCode::NoSuchFile);
        };
        let end = offset as usize + data.len();
        if full_at.is_some_and(|limit| end > limit) {
            return Err(StatusCode::Failure);
        }
        if bytes.len() < end {
            bytes.resize(end, 0);
        }
        bytes[offset as usize..end].copy_from_slice(&data);
        Ok(ok(id))
    }

    async fn lstat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        self.stat_path(id, &path)
    }

    async fn stat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        self.stat_path(id, &path)
    }

    async fn fstat(&mut self, id: u32, handle: String) -> Result<Attrs, Self::Error> {
        let path = self.handles.get(&handle).ok_or(StatusCode::Failure)?;
        self.stat_path(id, path)
    }

    async fn opendir(&mut self, id: u32, path: String) -> Result<Handle, Self::Error> {
        let path = absolute(&path);
        if self.disk.lock().unwrap().nodes.get(&path) != Some(&Node::Dir) {
            return Err(StatusCode::NoSuchFile);
        }
        Ok(self.handle(id, path))
    }

    async fn readdir(&mut self, id: u32, handle: String) -> Result<Name, Self::Error> {
        let dir = self
            .handles
            .get(&handle)
            .ok_or(StatusCode::Failure)?
            .clone();
        if !self.listed.insert(handle) {
            return Err(StatusCode::Eof);
        }
        let disk = self.disk.lock().unwrap();
        let prefix = if dir == "/" {
            "/".to_string()
        } else {
            format!("{dir}/")
        };
        let mut files: Vec<File> = disk
            .nodes
            .iter()
            .filter_map(|(path, node)| {
                let name = path.strip_prefix(&prefix)?;
                (!name.is_empty() && !name.contains('/')).then(|| File::new(name, attrs(node)))
            })
            .collect();
        files.extend(
            disk.rogue
                .iter()
                .filter(|(folder, _)| *folder == dir)
                .map(|(_, name)| File::new(name.as_str(), attrs(&file(b"rogue")))),
        );
        if files.is_empty() {
            return Err(StatusCode::Eof);
        }
        Ok(Name { id, files })
    }

    async fn remove(&mut self, id: u32, filename: String) -> Result<Status, Self::Error> {
        let mut disk = self.disk.lock().unwrap();
        match disk.nodes.remove(&absolute(&filename)) {
            Some(_) => Ok(ok(id)),
            None => Err(StatusCode::NoSuchFile),
        }
    }

    async fn mkdir(
        &mut self,
        id: u32,
        path: String,
        _attrs: FileAttributes,
    ) -> Result<Status, Self::Error> {
        let path = absolute(&path);
        let mut disk = self.disk.lock().unwrap();
        if disk.nodes.contains_key(&path) {
            return Err(StatusCode::Failure);
        }
        disk.nodes.insert(path, Node::Dir);
        Ok(ok(id))
    }

    async fn realpath(&mut self, id: u32, path: String) -> Result<Name, Self::Error> {
        Ok(Name {
            id,
            files: vec![File::dummy(absolute(&path))],
        })
    }

    async fn rename(
        &mut self,
        id: u32,
        oldpath: String,
        newpath: String,
    ) -> Result<Status, Self::Error> {
        let (old, new) = (absolute(&oldpath), absolute(&newpath));
        let mut disk = self.disk.lock().unwrap();
        if disk.nodes.contains_key(&new) {
            return Err(StatusCode::Failure);
        }
        let node = disk.nodes.remove(&old).ok_or(StatusCode::NoSuchFile)?;
        disk.nodes.insert(new, node);
        Ok(ok(id))
    }
}

/// An SFTP session on the fake host that keeps `disk`.
pub async fn connect(disk: &Shared) -> SftpSession {
    let (client, server) = tokio::io::duplex(1 << 20);
    let fake = Fake {
        disk: disk.clone(),
        handles: HashMap::new(),
        listed: HashSet::new(),
        next: 0,
    };
    russh_sftp::server::run(server, fake).await;
    SftpSession::new(client).await.expect("sftp session")
}
