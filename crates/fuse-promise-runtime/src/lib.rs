use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::path::{Component, Path, PathBuf};

pub const API_VERSION: u32 = 1;

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
}

#[derive(Debug, Default)]
pub struct Runtime {
    next_provider_id: u64,
    next_promise_id: u64,
    providers: BTreeMap<ProviderId, ProviderSession>,
    promises: BTreeMap<String, PromiseTree>,
}

impl Runtime {
    pub fn new() -> Self {
        Self {
            next_provider_id: 1,
            next_promise_id: 1,
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

    pub fn unregister_provider(&mut self, provider_id: ProviderId) {
        if let Some(provider) = self.providers.get_mut(&provider_id) {
            provider.state = ProviderState::Disconnected;
        }
        for promise in self.promises.values_mut() {
            if promise.provider_id == provider_id && promise.state == PromiseState::Available {
                promise.state = PromiseState::ProviderGone;
            }
        }
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

        let tree = builder.finish(promise_id.clone());
        self.promises.insert(promise_id, tree.clone());
        Ok(tree)
    }

    pub fn promise(&self, promise_id: &str) -> Option<&PromiseTree> {
        self.promises.get(promise_id)
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

fn validate_attr(kind: NodeKind, attr: NodeAttr) -> Result<()> {
    if attr.mode & !0o7777 != 0 {
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

        runtime.unregister_provider(provider);

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
    fn commit_fails_after_provider_unregisters() {
        let mut runtime = Runtime::new();
        let provider = runtime.register_provider();
        runtime.unregister_provider(provider);

        let builder = PromiseBuilder::new(provider);
        assert_eq!(runtime.commit_promise(builder), Err(Status::ProviderGone));
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
    }
}
