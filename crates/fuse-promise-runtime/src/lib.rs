use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::io;
use std::os::unix::fs::{DirBuilderExt, MetadataExt};
use std::path::{Component, Path, PathBuf};

pub const API_VERSION: u32 = 1;
pub const FUSE_ROOT_INODE: u64 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Ok,
    InvalidArgument,
    Unavailable,
    Permission,
    NotFound,
    AlreadyExists,
    ProviderGone,
    Io,
    Timeout,
    Cancelled,
    VersionMismatch,
}

impl Status {
    pub const fn as_str(self) -> &'static str {
        match self {
            Status::Ok => "ok",
            Status::InvalidArgument => "invalid argument",
            Status::Unavailable => "unavailable",
            Status::Permission => "permission denied",
            Status::NotFound => "not found",
            Status::AlreadyExists => "already exists",
            Status::ProviderGone => "provider gone",
            Status::Io => "io error",
            Status::Timeout => "timeout",
            Status::Cancelled => "cancelled",
            Status::VersionMismatch => "version mismatch",
        }
    }
}

pub type Result<T> = std::result::Result<T, Status>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ProviderId(u64);

impl ProviderId {
    pub fn from_raw(raw: u64) -> Option<Self> {
        if raw == 0 {
            None
        } else {
            Some(Self(raw))
        }
    }

    pub const fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct NodeId(u64);

impl NodeId {
    pub const fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderState {
    Live,
    Disconnected,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderSession {
    pub id: ProviderId,
    pub state: ProviderState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    File,
    Directory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NodeAttr {
    pub mode: u32,
    pub size: u64,
    pub mtime_nsec: i64,
}

impl NodeAttr {
    pub const fn new(mode: u32, size: u64, mtime_nsec: i64) -> Self {
        Self {
            mode,
            size,
            mtime_nsec,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromiseState {
    Available,
    ProviderGone,
    Materialized,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromiseNode {
    pub node_id: NodeId,
    pub inode: u64,
    pub relative_path: String,
    pub parent_path: Option<String>,
    pub name: String,
    pub provider_node_id: String,
    pub materialized_path: Option<String>,
    pub kind: NodeKind,
    pub attr: NodeAttr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromiseTree {
    pub promise_id: String,
    pub provider_id: ProviderId,
    pub state: PromiseState,
    pub nodes: BTreeMap<String, PromiseNode>,
    pub children: BTreeMap<String, BTreeSet<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadPlan {
    Request(ProviderReadPlan),
    Eof,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderReadPlan {
    pub provider_id: ProviderId,
    pub promise_id: String,
    pub relative_path: String,
    pub provider_node_id: String,
    pub offset: u64,
    pub length: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeEntry {
    MountRoot,
    PromiseNode {
        promise_id: String,
        node: PromiseNode,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectoryEntry {
    pub name: String,
    pub inode: u64,
    pub kind: NodeKind,
}

impl RuntimeEntry {
    pub fn inode(&self) -> u64 {
        match self {
            RuntimeEntry::MountRoot => FUSE_ROOT_INODE,
            RuntimeEntry::PromiseNode { node, .. } => node.inode,
        }
    }

    pub fn kind(&self) -> NodeKind {
        match self {
            RuntimeEntry::MountRoot => NodeKind::Directory,
            RuntimeEntry::PromiseNode { node, .. } => node.kind,
        }
    }
}

impl PromiseTree {
    pub fn get(&self, relative_path: &str) -> Option<&PromiseNode> {
        normalize_relative_path(relative_path)
            .ok()
            .and_then(|path| self.nodes.get(&path))
    }
}

#[derive(Debug, Clone)]
pub struct PromiseBuilder {
    provider_id: ProviderId,
    next_node_id: u64,
    next_inode: u64,
    nodes: BTreeMap<String, PromiseNode>,
    children: BTreeMap<String, BTreeSet<String>>,
}

impl PromiseBuilder {
    pub fn new(provider_id: ProviderId) -> Self {
        let mut nodes = BTreeMap::new();
        let mut children = BTreeMap::new();
        nodes.insert(
            String::new(),
            PromiseNode {
                node_id: NodeId(1),
                inode: 1,
                relative_path: String::new(),
                parent_path: None,
                name: String::new(),
                provider_node_id: String::new(),
                materialized_path: None,
                kind: NodeKind::Directory,
                attr: NodeAttr::new(0o755, 0, 0),
            },
        );
        children.insert(String::new(), BTreeSet::new());

        Self {
            provider_id,
            next_node_id: 2,
            next_inode: 2,
            nodes,
            children,
        }
    }

    pub fn add_dir(
        &mut self,
        relative_path: &str,
        attr: NodeAttr,
        provider_node_id: &str,
    ) -> Result<()> {
        self.add_node(relative_path, attr, provider_node_id, NodeKind::Directory)
    }

    pub fn add_file(
        &mut self,
        relative_path: &str,
        attr: NodeAttr,
        provider_node_id: &str,
    ) -> Result<()> {
        self.add_node(relative_path, attr, provider_node_id, NodeKind::File)
    }

    fn add_node(
        &mut self,
        relative_path: &str,
        attr: NodeAttr,
        provider_node_id: &str,
        kind: NodeKind,
    ) -> Result<()> {
        if provider_node_id.is_empty() {
            return Err(Status::InvalidArgument);
        }
        validate_attr(kind, attr)?;

        let path = normalize_relative_path(relative_path)?;
        if path.is_empty() {
            return Err(Status::InvalidArgument);
        }
        if self.nodes.contains_key(&path) {
            return Err(Status::AlreadyExists);
        }

        let parent = parent_path(&path);
        let Some(parent_node) = self.nodes.get(parent) else {
            return Err(Status::NotFound);
        };
        if parent_node.kind != NodeKind::Directory {
            return Err(Status::InvalidArgument);
        }

        let node_id = NodeId(self.next_node_id);
        self.next_node_id += 1;
        let inode = self.next_inode;
        self.next_inode += 1;
        let name = leaf_name(&path).to_owned();

        self.nodes.insert(
            path.clone(),
            PromiseNode {
                node_id,
                inode,
                relative_path: path.clone(),
                parent_path: Some(parent.to_owned()),
                name,
                provider_node_id: provider_node_id.to_owned(),
                materialized_path: None,
                kind,
                attr,
            },
        );
        self.children
            .entry(parent.to_owned())
            .or_default()
            .insert(path.clone());
        if kind == NodeKind::Directory {
            self.children.entry(path).or_default();
        }

        Ok(())
    }

    pub fn finish(self, promise_id: String) -> PromiseTree {
        PromiseTree {
            promise_id,
            provider_id: self.provider_id,
            state: PromiseState::Available,
            nodes: self.nodes,
            children: self.children,
        }
    }

    pub fn provider_id(&self) -> ProviderId {
        self.provider_id
    }

    pub fn nodes(&self) -> impl Iterator<Item = &PromiseNode> {
        self.nodes.values()
    }
}

#[derive(Debug, Default)]
pub struct Runtime {
    next_provider_id: u64,
    next_promise_id: u64,
    next_node_id: u64,
    next_inode: u64,
    providers: BTreeMap<ProviderId, ProviderSession>,
    promises: BTreeMap<String, PromiseTree>,
}

impl Runtime {
    pub fn new() -> Self {
        Self {
            next_provider_id: 1,
            next_promise_id: 1,
            next_node_id: 1,
            next_inode: FUSE_ROOT_INODE + 1,
            providers: BTreeMap::new(),
            promises: BTreeMap::new(),
        }
    }

    pub fn register_provider(&mut self) -> ProviderId {
        let provider_id = ProviderId(self.next_provider_id);
        self.next_provider_id += 1;
        self.providers.insert(
            provider_id,
            ProviderSession {
                id: provider_id,
                state: ProviderState::Live,
            },
        );
        provider_id
    }

    pub fn unregister_provider(&mut self, provider_id: ProviderId) -> Result<()> {
        let Some(provider) = self.providers.get_mut(&provider_id) else {
            return Err(Status::NotFound);
        };

        provider.state = ProviderState::Disconnected;
        for promise in self.promises.values_mut() {
            if promise.provider_id == provider_id && promise.state == PromiseState::Available {
                promise.state = PromiseState::ProviderGone;
            }
        }

        Ok(())
    }

    pub fn has_provider(&self, provider_id: ProviderId) -> bool {
        self.providers
            .get(&provider_id)
            .is_some_and(|provider| provider.state == ProviderState::Live)
    }

    pub fn provider(&self, provider_id: ProviderId) -> Option<&ProviderSession> {
        self.providers.get(&provider_id)
    }

    pub fn commit_promise(&mut self, builder: PromiseBuilder) -> Result<PromiseTree> {
        if !self.has_provider(builder.provider_id) {
            return Err(Status::ProviderGone);
        }

        let promise_id = format!("promise-{}", self.next_promise_id);
        self.next_promise_id += 1;

        let mut tree = builder.finish(promise_id.clone());
        self.assign_runtime_ids(&mut tree)?;
        self.promises.insert(promise_id, tree.clone());
        Ok(tree)
    }

    pub fn promise(&self, promise_id: &str) -> Option<&PromiseTree> {
        self.promises.get(promise_id)
    }

    pub fn lookup_inode(&self, inode: u64) -> Result<RuntimeEntry> {
        if inode == FUSE_ROOT_INODE {
            return Ok(RuntimeEntry::MountRoot);
        }

        self.promises
            .values()
            .find_map(|tree| {
                tree.nodes.values().find_map(|node| {
                    (node.inode == inode).then(|| RuntimeEntry::PromiseNode {
                        promise_id: tree.promise_id.clone(),
                        node: node.clone(),
                    })
                })
            })
            .ok_or(Status::NotFound)
    }

    pub fn lookup_child(&self, parent_inode: u64, name: &str) -> Result<RuntimeEntry> {
        validate_child_name(name)?;

        if parent_inode == FUSE_ROOT_INODE {
            let tree = self.promise(name).ok_or(Status::NotFound)?;
            let node = tree.nodes.get("").ok_or(Status::NotFound)?;
            return Ok(RuntimeEntry::PromiseNode {
                promise_id: tree.promise_id.clone(),
                node: node.clone(),
            });
        }

        let RuntimeEntry::PromiseNode {
            promise_id,
            node: parent,
        } = self.lookup_inode(parent_inode)?
        else {
            unreachable!("mount root was handled before inode lookup");
        };
        if parent.kind != NodeKind::Directory {
            return Err(Status::InvalidArgument);
        }

        let tree = self.promise(&promise_id).ok_or(Status::NotFound)?;
        let child_relative_path = child_path(&parent.relative_path, name);
        if !tree
            .children
            .get(&parent.relative_path)
            .is_some_and(|children| children.contains(&child_relative_path))
        {
            return Err(Status::NotFound);
        }

        let child = tree
            .nodes
            .get(&child_relative_path)
            .ok_or(Status::NotFound)?;
        Ok(RuntimeEntry::PromiseNode {
            promise_id,
            node: child.clone(),
        })
    }

    pub fn read_dir(&self, inode: u64) -> Result<Vec<DirectoryEntry>> {
        if inode == FUSE_ROOT_INODE {
            return self
                .promises
                .values()
                .map(|tree| {
                    let root = tree.nodes.get("").ok_or(Status::NotFound)?;
                    Ok(DirectoryEntry {
                        name: tree.promise_id.clone(),
                        inode: root.inode,
                        kind: NodeKind::Directory,
                    })
                })
                .collect();
        }

        let RuntimeEntry::PromiseNode { promise_id, node } = self.lookup_inode(inode)? else {
            unreachable!("mount root was handled before inode lookup");
        };
        if node.kind != NodeKind::Directory {
            return Err(Status::InvalidArgument);
        }

        let tree = self.promise(&promise_id).ok_or(Status::NotFound)?;
        tree.children
            .get(&node.relative_path)
            .ok_or(Status::NotFound)?
            .iter()
            .map(|child_path| {
                let child = tree.nodes.get(child_path).ok_or(Status::NotFound)?;
                Ok(DirectoryEntry {
                    name: child.name.clone(),
                    inode: child.inode,
                    kind: child.kind,
                })
            })
            .collect()
    }

    pub fn plan_read(
        &self,
        promise_id: &str,
        relative_path: &str,
        offset: u64,
        length: u32,
    ) -> Result<ReadPlan> {
        let tree = self.promise(promise_id).ok_or(Status::NotFound)?;
        if tree.state != PromiseState::Available {
            return Err(Status::ProviderGone);
        }
        if !self.has_provider(tree.provider_id) {
            return Err(Status::ProviderGone);
        }

        let node = tree.get(relative_path).ok_or(Status::NotFound)?;
        if node.kind != NodeKind::File {
            return Err(Status::InvalidArgument);
        }
        if length == 0 || offset >= node.attr.size {
            return Ok(ReadPlan::Eof);
        }

        let remaining = node.attr.size - offset;
        let length = u64::from(length).min(remaining);
        let length = u32::try_from(length).map_err(|_| Status::InvalidArgument)?;
        if offset.checked_add(u64::from(length)).is_none() {
            return Err(Status::InvalidArgument);
        }

        Ok(ReadPlan::Request(ProviderReadPlan {
            provider_id: tree.provider_id,
            promise_id: tree.promise_id.clone(),
            relative_path: node.relative_path.clone(),
            provider_node_id: node.provider_node_id.clone(),
            offset,
            length,
        }))
    }

    pub fn mark_node_materialized(
        &mut self,
        promise_id: &str,
        relative_path: &str,
        materialized_path: &Path,
    ) -> Result<()> {
        let normalized_path = normalize_relative_path(relative_path)?;
        let tree = self.promises.get_mut(promise_id).ok_or(Status::NotFound)?;
        let node = tree
            .nodes
            .get_mut(&normalized_path)
            .ok_or(Status::NotFound)?;
        if node.kind != NodeKind::File {
            return Err(Status::InvalidArgument);
        }
        let Some(path) = materialized_path.to_str() else {
            return Err(Status::InvalidArgument);
        };
        node.materialized_path = Some(path.to_owned());
        Ok(())
    }

    pub fn promises(&self) -> impl Iterator<Item = &PromiseTree> {
        self.promises.values()
    }

    pub fn provider_count(&self) -> usize {
        self.providers.len()
    }

    pub fn promise_count(&self) -> usize {
        self.promises.len()
    }

    fn assign_runtime_ids(&mut self, tree: &mut PromiseTree) -> Result<()> {
        for node in tree.nodes.values_mut() {
            node.node_id = self.allocate_node_id()?;
            node.inode = self.allocate_inode()?;
        }

        Ok(())
    }

    fn allocate_node_id(&mut self) -> Result<NodeId> {
        let raw = self.next_node_id;
        self.next_node_id = self
            .next_node_id
            .checked_add(1)
            .ok_or(Status::InvalidArgument)?;
        Ok(NodeId(raw))
    }

    fn allocate_inode(&mut self) -> Result<u64> {
        let raw = self.next_inode;
        self.next_inode = self
            .next_inode
            .checked_add(1)
            .ok_or(Status::InvalidArgument)?;
        Ok(raw)
    }
}

pub fn default_mount_path() -> Result<PathBuf> {
    let Some(runtime_dir) = env::var_os("XDG_RUNTIME_DIR") else {
        return Err(Status::Unavailable);
    };
    let runtime_dir = PathBuf::from(runtime_dir);
    validate_runtime_dir_path(&runtime_dir)?;

    Ok(runtime_dir.join("fuse-promise"))
}

pub fn default_control_socket_path() -> Result<PathBuf> {
    let Some(runtime_dir) = env::var_os("XDG_RUNTIME_DIR") else {
        return Err(Status::Unavailable);
    };
    let runtime_dir = PathBuf::from(runtime_dir);
    validate_runtime_dir_path(&runtime_dir)?;

    Ok(runtime_dir.join("fuse-promise.sock"))
}

pub fn validate_runtime_dir_path(runtime_dir: &Path) -> Result<()> {
    if !runtime_dir.is_absolute() {
        return Err(Status::InvalidArgument);
    }

    let metadata = fs::metadata(runtime_dir).map_err(|_| Status::Unavailable)?;
    if !metadata.is_dir() {
        return Err(Status::InvalidArgument);
    }
    if metadata.uid() != rustix::process::getuid().as_raw() {
        return Err(Status::Permission);
    }
    if metadata.mode() & 0o077 != 0 {
        return Err(Status::Permission);
    }

    Ok(())
}

pub fn prepare_mount_dir(mount_path: &Path) -> Result<()> {
    if !mount_path.is_absolute() {
        return Err(Status::InvalidArgument);
    }

    match fs::symlink_metadata(mount_path) {
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let mut builder = fs::DirBuilder::new();
            builder.mode(0o700);
            builder
                .create(mount_path)
                .map_err(|error| match error.kind() {
                    io::ErrorKind::PermissionDenied => Status::Permission,
                    io::ErrorKind::AlreadyExists => Status::AlreadyExists,
                    _ => Status::Io,
                })?;
        }
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
            return Err(Status::Permission);
        }
        Err(_) => return Err(Status::Unavailable),
    }

    validate_mount_dir_path(mount_path)
}

pub fn validate_mount_dir_path(mount_path: &Path) -> Result<()> {
    if !mount_path.is_absolute() {
        return Err(Status::InvalidArgument);
    }

    let metadata = fs::symlink_metadata(mount_path).map_err(|_| Status::Unavailable)?;
    if !metadata.is_dir() {
        return Err(Status::InvalidArgument);
    }
    if metadata.uid() != rustix::process::getuid().as_raw() {
        return Err(Status::Permission);
    }
    if metadata.mode() & 0o077 != 0 {
        return Err(Status::Permission);
    }

    Ok(())
}

pub fn normalize_relative_path(input: &str) -> Result<String> {
    if input.as_bytes().contains(&0) {
        return Err(Status::InvalidArgument);
    }

    let path = Path::new(input);
    if path.is_absolute() {
        return Err(Status::InvalidArgument);
    }

    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => {
                let Some(part) = part.to_str() else {
                    return Err(Status::InvalidArgument);
                };
                if part.is_empty() {
                    return Err(Status::InvalidArgument);
                }
                parts.push(part.to_owned());
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(Status::InvalidArgument);
            }
        }
    }

    Ok(parts.join("/"))
}

fn parent_path(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(parent, _)| parent)
}

fn leaf_name(path: &str) -> &str {
    path.rsplit_once('/').map_or(path, |(_, name)| name)
}

fn validate_child_name(name: &str) -> Result<()> {
    if name.is_empty() || name == "." || name == ".." {
        return Err(Status::InvalidArgument);
    }
    if name.as_bytes().contains(&0) || name.contains('/') {
        return Err(Status::InvalidArgument);
    }

    Ok(())
}

fn child_path(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_owned()
    } else {
        format!("{parent}/{name}")
    }
}

fn validate_attr(kind: NodeKind, attr: NodeAttr) -> Result<()> {
    if attr.mode & !0o7777 != 0 {
        return Err(Status::InvalidArgument);
    }
    if attr.mtime_nsec < 0 {
        return Err(Status::InvalidArgument);
    }
    if kind == NodeKind::Directory && attr.size != 0 {
        return Err(Status::InvalidArgument);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn normalizes_safe_relative_paths() {
        assert_eq!(normalize_relative_path("./a/b").unwrap(), "a/b");
        assert_eq!(normalize_relative_path("a//b").unwrap(), "a/b");
    }

    #[test]
    fn rejects_unsafe_paths() {
        assert_eq!(
            normalize_relative_path("/a").unwrap_err(),
            Status::InvalidArgument
        );
        assert_eq!(
            normalize_relative_path("../a").unwrap_err(),
            Status::InvalidArgument
        );
        assert_eq!(
            normalize_relative_path("a/../b").unwrap_err(),
            Status::InvalidArgument
        );
    }

    #[test]
    fn builder_requires_declared_parent_directories() {
        let mut runtime = Runtime::new();
        let provider = runtime.register_provider();
        let mut builder = PromiseBuilder::new(provider);
        let attr = NodeAttr::new(0o644, 4, 0);

        assert_eq!(
            builder.add_file("missing/file.txt", attr, "node-1"),
            Err(Status::NotFound)
        );

        builder
            .add_dir("missing", NodeAttr::new(0o755, 0, 0), "dir-1")
            .unwrap();
        builder
            .add_file("missing/file.txt", attr, "node-1")
            .unwrap();

        let tree = runtime.commit_promise(builder).unwrap();
        assert_eq!(tree.promise_id, "promise-1");
        assert_eq!(tree.state, PromiseState::Available);
        assert!(tree.get("missing/file.txt").is_some());
        assert_eq!(
            tree.children
                .get("missing")
                .unwrap()
                .iter()
                .cloned()
                .collect::<Vec<_>>(),
            vec!["missing/file.txt".to_owned()]
        );
    }

    #[test]
    fn provider_disconnect_marks_existing_promises_unavailable() {
        let mut runtime = Runtime::new();
        let provider = runtime.register_provider();
        let builder = PromiseBuilder::new(provider);
        let tree = runtime.commit_promise(builder).unwrap();

        runtime.unregister_provider(provider).unwrap();

        assert_eq!(
            runtime.provider(provider).unwrap().state,
            ProviderState::Disconnected
        );
        assert_eq!(
            runtime.promise(&tree.promise_id).unwrap().state,
            PromiseState::ProviderGone
        );
    }

    #[test]
    fn commit_assigns_daemon_global_node_ids_and_inodes() {
        let mut runtime = Runtime::new();
        let provider = runtime.register_provider();

        let first = runtime
            .commit_promise(sample_file_builder(provider, "remote-file-1"))
            .unwrap();
        let second = runtime
            .commit_promise(sample_file_builder(provider, "remote-file-2"))
            .unwrap();

        let mut node_ids = BTreeSet::new();
        let mut inodes = BTreeSet::new();
        for tree in [&first, &second] {
            for node in tree.nodes.values() {
                assert!(node_ids.insert(node.node_id.raw()));
                assert!(inodes.insert(node.inode));
                assert_ne!(node.inode, FUSE_ROOT_INODE);
            }
        }

        assert_eq!(node_ids, BTreeSet::from([1, 2, 3, 4, 5, 6]));
        assert_eq!(inodes, BTreeSet::from([2, 3, 4, 5, 6, 7]));
    }

    #[test]
    fn resolves_runtime_entries_by_inode_and_child_name() {
        let (runtime, _) = runtime_with_file();

        let root_entries = runtime.read_dir(FUSE_ROOT_INODE).unwrap();
        assert_eq!(root_entries.len(), 1);
        assert_eq!(root_entries[0].name, "promise-1");
        assert_eq!(root_entries[0].kind, NodeKind::Directory);

        let RuntimeEntry::PromiseNode {
            promise_id,
            node: promise_root,
        } = runtime.lookup_child(FUSE_ROOT_INODE, "promise-1").unwrap()
        else {
            panic!("promise id lookup should return a promise node");
        };
        assert_eq!(promise_id, "promise-1");
        assert_eq!(promise_root.relative_path, "");

        let RuntimeEntry::PromiseNode { node: docs, .. } =
            runtime.lookup_child(promise_root.inode, "docs").unwrap()
        else {
            panic!("directory lookup should return a promise node");
        };
        assert_eq!(docs.relative_path, "docs");
        assert_eq!(docs.kind, NodeKind::Directory);

        let docs_entries = runtime.read_dir(docs.inode).unwrap();
        assert_eq!(
            docs_entries,
            vec![DirectoryEntry {
                name: "readme.txt".to_owned(),
                inode: 4,
                kind: NodeKind::File,
            }]
        );

        let RuntimeEntry::PromiseNode { node: file, .. } =
            runtime.lookup_child(docs.inode, "readme.txt").unwrap()
        else {
            panic!("file lookup should return a promise node");
        };
        assert_eq!(
            runtime.lookup_inode(file.inode).unwrap().inode(),
            file.inode
        );
        assert_eq!(runtime.read_dir(file.inode), Err(Status::InvalidArgument));
        assert_eq!(
            runtime.lookup_child(FUSE_ROOT_INODE, "bad/name"),
            Err(Status::InvalidArgument)
        );
    }

    #[test]
    fn commit_fails_after_provider_unregisters() {
        let mut runtime = Runtime::new();
        let provider = runtime.register_provider();
        runtime.unregister_provider(provider).unwrap();

        let builder = PromiseBuilder::new(provider);
        assert_eq!(runtime.commit_promise(builder), Err(Status::ProviderGone));
    }

    #[test]
    fn unregister_rejects_unknown_provider() {
        let mut runtime = Runtime::new();
        let provider = ProviderId::from_raw(99).unwrap();

        assert_eq!(runtime.unregister_provider(provider), Err(Status::NotFound));
    }

    #[test]
    fn plans_provider_owned_file_reads() {
        let (runtime, provider) = runtime_with_file();

        let plan = runtime
            .plan_read("promise-1", "docs/readme.txt", 2, 8)
            .unwrap();

        assert_eq!(
            plan,
            ReadPlan::Request(ProviderReadPlan {
                provider_id: provider,
                promise_id: "promise-1".to_owned(),
                relative_path: "docs/readme.txt".to_owned(),
                provider_node_id: "remote-file-1".to_owned(),
                offset: 2,
                length: 8,
            })
        );
    }

    #[test]
    fn read_planning_caps_at_eof() {
        let (runtime, provider) = runtime_with_file();

        assert_eq!(
            runtime
                .plan_read("promise-1", "docs/readme.txt", 10, 8)
                .unwrap(),
            ReadPlan::Request(ProviderReadPlan {
                provider_id: provider,
                promise_id: "promise-1".to_owned(),
                relative_path: "docs/readme.txt".to_owned(),
                provider_node_id: "remote-file-1".to_owned(),
                offset: 10,
                length: 2,
            })
        );
        assert_eq!(
            runtime
                .plan_read("promise-1", "docs/readme.txt", 12, 8)
                .unwrap(),
            ReadPlan::Eof
        );
        assert_eq!(
            runtime
                .plan_read("promise-1", "docs/readme.txt", 0, 0)
                .unwrap(),
            ReadPlan::Eof
        );
    }

    #[test]
    fn read_planning_rejects_missing_or_non_file_nodes() {
        let (runtime, _) = runtime_with_file();

        assert_eq!(
            runtime.plan_read("promise-1", "docs/missing.txt", 0, 1),
            Err(Status::NotFound)
        );
        assert_eq!(
            runtime.plan_read("promise-1", "docs", 0, 1),
            Err(Status::InvalidArgument)
        );
        assert_eq!(
            runtime.plan_read("missing-promise", "docs/readme.txt", 0, 1),
            Err(Status::NotFound)
        );
    }

    #[test]
    fn read_planning_rejects_disconnected_provider() {
        let (mut runtime, _) = runtime_with_file();

        runtime
            .unregister_provider(ProviderId::from_raw(1).unwrap())
            .unwrap();

        assert_eq!(
            runtime.plan_read("promise-1", "docs/readme.txt", 0, 1),
            Err(Status::ProviderGone)
        );
    }

    #[test]
    fn validates_safe_runtime_directory() {
        let dir = tempfile::tempdir().unwrap();
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();

        assert_eq!(validate_runtime_dir_path(dir.path()), Ok(()));
    }

    #[test]
    fn rejects_unsafe_runtime_directory_paths() {
        assert_eq!(
            validate_runtime_dir_path(Path::new("relative")),
            Err(Status::InvalidArgument)
        );

        let dir = tempfile::tempdir().unwrap();
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            validate_runtime_dir_path(dir.path()),
            Err(Status::Permission)
        );

        let file = tempfile::NamedTempFile::new().unwrap();
        assert_eq!(
            validate_runtime_dir_path(file.path()),
            Err(Status::InvalidArgument)
        );
    }

    #[test]
    fn prepares_private_mount_directory() {
        let dir = tempfile::tempdir().unwrap();
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let mount_path = dir.path().join("fuse-promise");

        assert_eq!(prepare_mount_dir(&mount_path), Ok(()));

        let metadata = fs::symlink_metadata(&mount_path).unwrap();
        assert!(metadata.is_dir());
        assert_eq!(metadata.mode() & 0o777, 0o700);
        assert_eq!(metadata.uid(), rustix::process::getuid().as_raw());
    }

    #[test]
    fn rejects_unsafe_mount_directory_paths() {
        assert_eq!(
            prepare_mount_dir(Path::new("relative")),
            Err(Status::InvalidArgument)
        );

        let dir = tempfile::tempdir().unwrap();
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();

        let unsafe_mount = dir.path().join("unsafe");
        fs::create_dir(&unsafe_mount).unwrap();
        fs::set_permissions(&unsafe_mount, fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(prepare_mount_dir(&unsafe_mount), Err(Status::Permission));

        let file_mount = dir.path().join("file");
        fs::write(&file_mount, b"not a directory").unwrap();
        assert_eq!(prepare_mount_dir(&file_mount), Err(Status::InvalidArgument));

        let symlink_mount = dir.path().join("symlink");
        std::os::unix::fs::symlink(dir.path(), &symlink_mount).unwrap();
        assert_eq!(
            prepare_mount_dir(&symlink_mount),
            Err(Status::InvalidArgument)
        );
    }

    #[test]
    fn validates_mode_and_directory_size() {
        let mut runtime = Runtime::new();
        let provider = runtime.register_provider();
        let mut builder = PromiseBuilder::new(provider);

        assert_eq!(
            builder.add_file("bad", NodeAttr::new(0o100644, 1, 0), "bad"),
            Err(Status::InvalidArgument)
        );
        assert_eq!(
            builder.add_dir("nonempty-dir", NodeAttr::new(0o755, 1, 0), "dir"),
            Err(Status::InvalidArgument)
        );
        assert_eq!(
            builder.add_file("negative-mtime", NodeAttr::new(0o644, 1, -1), "bad"),
            Err(Status::InvalidArgument)
        );
    }

    fn runtime_with_file() -> (Runtime, ProviderId) {
        let mut runtime = Runtime::new();
        let provider = runtime.register_provider();
        let builder = sample_file_builder(provider, "remote-file-1");
        runtime.commit_promise(builder).unwrap();

        (runtime, provider)
    }

    fn sample_file_builder(provider: ProviderId, remote_file: &str) -> PromiseBuilder {
        let mut builder = PromiseBuilder::new(provider);
        builder
            .add_dir("docs", NodeAttr::new(0o755, 0, 0), "remote-dir-1")
            .unwrap();
        builder
            .add_file("docs/readme.txt", NodeAttr::new(0o644, 12, 0), remote_file)
            .unwrap();
        builder
    }
}
