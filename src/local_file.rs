use std::ffi::{CStr, CString};
use std::fs::File;
use std::io::{self, Write};
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use thiserror::Error;

static TEMPORARY_SEQUENCE: AtomicU64 = AtomicU64::new(0);
const TEMPORARY_ATTEMPTS: usize = 128;
pub(crate) const MAX_LOCAL_PATH_BYTES: usize = 4_096;
pub(crate) const MAX_LOCAL_PATH_COMPONENTS: usize = 64;
pub(crate) const MAX_LOCAL_COMPONENT_BYTES: usize = 255;

pub(crate) type Result<T> = std::result::Result<T, LocalFileError>;

#[derive(Debug, Error)]
pub(crate) enum LocalFileError {
    #[error("invalid local path: {0}")]
    InvalidPath(&'static str),
    #[error("a local path component is a symbolic link")]
    Symlink,
    #[error("a local path component is not a directory")]
    NotDirectory,
    #[error("the download destination already exists")]
    DestinationExists,
    #[error("the download destination parent changed before commit")]
    ParentChanged,
    #[error("download exceeds the configured {limit}-byte limit")]
    SizeLimit { limit: u64 },
    #[error("local file operation {operation} failed")]
    Io {
        operation: &'static str,
        #[source]
        source: io::Error,
    },
}

impl LocalFileError {
    fn io(operation: &'static str, source: io::Error) -> Self {
        Self::Io { operation, source }
    }
}

impl From<LocalFileError> for crate::error::Error {
    fn from(error: LocalFileError) -> Self {
        Self::LocalFile {
            operation: error.to_string(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DownloadDurability {
    Synced,
    DirectorySyncWarning,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DownloadCommit {
    pub(crate) bytes_written: u64,
    pub(crate) durability: DownloadDurability,
}

/// An MCP file-system capability rooted at a directory descriptor opened once
/// during server startup.
pub(crate) struct McpFileRoot {
    directory: File,
}

impl McpFileRoot {
    pub(crate) fn open(path: &Path) -> Result<Self> {
        let components = parse_absolute(path, true)?;
        let root = open_anchor(ROOT)?;
        let directory = open_directories(&root, &components)?;
        Ok(Self { directory })
    }

    pub(crate) fn prepare_download(
        &self,
        relative_path: &Path,
        byte_limit: u64,
    ) -> Result<BoundedDownload> {
        let components = parse_relative(relative_path, false)?;
        let anchor = self
            .directory
            .try_clone()
            .map_err(|error| LocalFileError::io("clone file root", error))?;
        prepare_download(anchor, components, byte_limit)
    }
}

pub(crate) fn validate_mcp_download_path(path: &Path) -> Result<()> {
    parse_relative(path, false).map(|_| ())
}

pub(crate) fn validate_cli_download_path(path: &Path) -> Result<()> {
    if path.is_absolute() {
        parse_absolute(path, false)
    } else {
        parse_relative(path, false)
    }
    .map(|_| ())
}

/// Prepare a direct CLI download. Absolute paths are anchored at an opened `/`
/// descriptor; relative paths are anchored at an opened current-directory
/// descriptor.
pub(crate) fn prepare_cli_download(path: &Path, byte_limit: u64) -> Result<BoundedDownload> {
    let (anchor_name, components) = if path.is_absolute() {
        (ROOT, parse_absolute(path, false)?)
    } else {
        (CURRENT_DIRECTORY, parse_relative(path, false)?)
    };
    let anchor = open_anchor(anchor_name)?;
    prepare_download(anchor, components, byte_limit)
}

/// A bounded, descriptor-anchored download that owns its temporary file.
///
/// Dropping this value before a successful commit unlinks the temporary file.
pub(crate) struct BoundedDownload {
    anchor: File,
    parent: File,
    parent_components: Vec<CString>,
    destination: CString,
    temporary: CString,
    file: Option<File>,
    byte_limit: u64,
    bytes_written: u64,
    committed: bool,
    #[cfg(test)]
    force_directory_sync_failure: bool,
}

impl BoundedDownload {
    pub(crate) fn bytes_written(&self) -> u64 {
        self.bytes_written
    }

    pub(crate) fn write_chunk(&mut self, bytes: &[u8]) -> Result<()> {
        let length = u64::try_from(bytes.len()).map_err(|_| LocalFileError::SizeLimit {
            limit: self.byte_limit,
        })?;
        let new_length =
            self.bytes_written
                .checked_add(length)
                .ok_or(LocalFileError::SizeLimit {
                    limit: self.byte_limit,
                })?;
        if new_length > self.byte_limit {
            return Err(LocalFileError::SizeLimit {
                limit: self.byte_limit,
            });
        }

        self.file
            .as_mut()
            .expect("an uncommitted download always owns its temporary file")
            .write_all(bytes)
            .map_err(|error| LocalFileError::io("write download", error))?;
        self.bytes_written = new_length;
        Ok(())
    }

    pub(crate) fn commit(mut self) -> Result<DownloadCommit> {
        let file = self
            .file
            .as_mut()
            .expect("an uncommitted download always owns its temporary file");
        file.flush()
            .map_err(|error| LocalFileError::io("flush download", error))?;
        file.sync_all()
            .map_err(|error| LocalFileError::io("sync download", error))?;

        let verified_parent = open_directories(&self.anchor, &self.parent_components)?;
        if directory_identity(&verified_parent)? != directory_identity(&self.parent)? {
            return Err(LocalFileError::ParentChanged);
        }

        rename_no_replace(
            self.parent.as_raw_fd(),
            &self.temporary,
            verified_parent.as_raw_fd(),
            &self.destination,
        )?;
        self.committed = true;

        let directory_sync = {
            #[cfg(test)]
            {
                if self.force_directory_sync_failure {
                    Err(io::Error::other("injected directory sync failure"))
                } else {
                    verified_parent.sync_all()
                }
            }
            #[cfg(not(test))]
            {
                verified_parent.sync_all()
            }
        };

        Ok(DownloadCommit {
            bytes_written: self.bytes_written,
            durability: if directory_sync.is_ok() {
                DownloadDurability::Synced
            } else {
                DownloadDurability::DirectorySyncWarning
            },
        })
    }

    #[cfg(test)]
    fn inject_directory_sync_failure(&mut self) {
        self.force_directory_sync_failure = true;
    }
}

impl Drop for BoundedDownload {
    fn drop(&mut self) {
        if !self.committed {
            // The descriptor keeps cleanup anchored even if the visible path
            // was replaced. Ignore cleanup errors because Drop cannot report
            // them and the requested destination has not been committed.
            unsafe {
                libc::unlinkat(self.parent.as_raw_fd(), self.temporary.as_ptr(), 0);
            }
        }
    }
}

fn prepare_download(
    anchor: File,
    mut components: Vec<CString>,
    byte_limit: u64,
) -> Result<BoundedDownload> {
    let destination = components
        .pop()
        .ok_or(LocalFileError::InvalidPath("a file name is required"))?;
    let parent = open_directories(&anchor, &components)?;
    ensure_destination_absent(&parent, &destination)?;
    let (temporary, file) = create_temporary(&parent)?;

    Ok(BoundedDownload {
        anchor,
        parent,
        parent_components: components,
        destination,
        temporary,
        file: Some(file),
        byte_limit,
        bytes_written: 0,
        committed: false,
        #[cfg(test)]
        force_directory_sync_failure: false,
    })
}

const ROOT: &CStr = c"/";
const CURRENT_DIRECTORY: &CStr = c".";

fn open_anchor(path: &CStr) -> Result<File> {
    let descriptor = unsafe {
        libc::open(
            path.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
        )
    };
    if descriptor < 0 {
        return Err(LocalFileError::io(
            "open path anchor",
            io::Error::last_os_error(),
        ));
    }

    Ok(unsafe { File::from_raw_fd(descriptor) })
}

fn open_directories(anchor: &File, components: &[CString]) -> Result<File> {
    let mut directory = anchor
        .try_clone()
        .map_err(|error| LocalFileError::io("clone directory", error))?;
    for component in components {
        directory = open_directory_at(directory.as_raw_fd(), component)?;
    }
    Ok(directory)
}

fn open_directory_at(parent: RawFd, name: &CStr) -> Result<File> {
    let descriptor = unsafe {
        libc::openat(
            parent,
            name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if descriptor < 0 {
        let error = io::Error::last_os_error();
        return match error.raw_os_error() {
            Some(libc::ELOOP) => Err(LocalFileError::Symlink),
            Some(libc::ENOTDIR) => Err(LocalFileError::NotDirectory),
            _ => Err(LocalFileError::io("open path directory", error)),
        };
    }

    let directory = unsafe { File::from_raw_fd(descriptor) };
    let identity = file_identity(&directory)?;
    if !identity.is_directory {
        return Err(LocalFileError::NotDirectory);
    }
    Ok(directory)
}

fn ensure_destination_absent(parent: &File, destination: &CStr) -> Result<()> {
    let mut metadata = std::mem::MaybeUninit::<libc::stat>::uninit();
    let result = unsafe {
        libc::fstatat(
            parent.as_raw_fd(),
            destination.as_ptr(),
            metadata.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if result == 0 {
        let metadata = unsafe { metadata.assume_init() };
        if metadata.st_mode & libc::S_IFMT == libc::S_IFLNK {
            return Err(LocalFileError::Symlink);
        }
        return Err(LocalFileError::DestinationExists);
    }

    let error = io::Error::last_os_error();
    if error.kind() == io::ErrorKind::NotFound {
        Ok(())
    } else {
        Err(LocalFileError::io("inspect download destination", error))
    }
}

fn create_temporary(parent: &File) -> Result<(CString, File)> {
    let process = std::process::id();
    for _ in 0..TEMPORARY_ATTEMPTS {
        let sequence = TEMPORARY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let name = CString::new(format!(".lurkline-download-{process}-{sequence}.tmp"))
            .expect("generated temporary names do not contain NUL");
        let descriptor = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                0o600,
            )
        };
        if descriptor >= 0 {
            let file = unsafe { File::from_raw_fd(descriptor) };
            let chmod_result = unsafe { libc::fchmod(file.as_raw_fd(), 0o600) };
            if chmod_result != 0 {
                let error = io::Error::last_os_error();
                unsafe {
                    libc::unlinkat(parent.as_raw_fd(), name.as_ptr(), 0);
                }
                return Err(LocalFileError::io("set download permissions", error));
            }
            let identity = match file_identity(&file) {
                Ok(identity) => identity,
                Err(error) => {
                    unsafe {
                        libc::unlinkat(parent.as_raw_fd(), name.as_ptr(), 0);
                    }
                    return Err(error);
                }
            };
            if !identity.is_regular {
                unsafe {
                    libc::unlinkat(parent.as_raw_fd(), name.as_ptr(), 0);
                }
                return Err(LocalFileError::io(
                    "create download temporary file",
                    io::Error::other("temporary file is not regular"),
                ));
            }
            return Ok((name, file));
        }

        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::AlreadyExists {
            return Err(LocalFileError::io("create download temporary file", error));
        }
    }

    Err(LocalFileError::io(
        "create download temporary file",
        io::Error::new(
            io::ErrorKind::AlreadyExists,
            "temporary name attempts exhausted",
        ),
    ))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FileIdentity {
    device: libc::dev_t,
    inode: libc::ino_t,
    is_directory: bool,
    is_regular: bool,
}

fn file_identity(file: &File) -> Result<FileIdentity> {
    let mut metadata = std::mem::MaybeUninit::<libc::stat>::uninit();
    let result = unsafe { libc::fstat(file.as_raw_fd(), metadata.as_mut_ptr()) };
    if result != 0 {
        return Err(LocalFileError::io(
            "inspect file descriptor",
            io::Error::last_os_error(),
        ));
    }
    let metadata = unsafe { metadata.assume_init() };
    let kind = metadata.st_mode & libc::S_IFMT;
    Ok(FileIdentity {
        device: metadata.st_dev,
        inode: metadata.st_ino,
        is_directory: kind == libc::S_IFDIR,
        is_regular: kind == libc::S_IFREG,
    })
}

fn directory_identity(file: &File) -> Result<(libc::dev_t, libc::ino_t)> {
    let identity = file_identity(file)?;
    if !identity.is_directory {
        return Err(LocalFileError::NotDirectory);
    }
    Ok((identity.device, identity.inode))
}

#[cfg(target_os = "macos")]
fn rename_no_replace(
    source_directory: RawFd,
    source: &CStr,
    destination_directory: RawFd,
    destination: &CStr,
) -> Result<()> {
    let result = unsafe {
        libc::renameatx_np(
            source_directory,
            source.as_ptr(),
            destination_directory,
            destination.as_ptr(),
            libc::RENAME_EXCL,
        )
    };
    map_rename_result(result)
}

#[cfg(target_os = "linux")]
fn rename_no_replace(
    source_directory: RawFd,
    source: &CStr,
    destination_directory: RawFd,
    destination: &CStr,
) -> Result<()> {
    let result = unsafe {
        libc::renameat2(
            source_directory,
            source.as_ptr(),
            destination_directory,
            destination.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    map_rename_result(result)
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn map_rename_result(result: libc::c_int) -> Result<()> {
    if result == 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    if error.kind() == io::ErrorKind::AlreadyExists {
        Err(LocalFileError::DestinationExists)
    } else {
        Err(LocalFileError::io("commit download", error))
    }
}

fn parse_absolute(path: &Path, allow_root: bool) -> Result<Vec<CString>> {
    let bytes = path.as_os_str().as_bytes();
    validate_total_path_length(bytes)?;
    if !bytes.starts_with(b"/") {
        return Err(LocalFileError::InvalidPath("an absolute path is required"));
    }
    parse_components(&bytes[1..], allow_root)
}

fn parse_relative(path: &Path, allow_empty: bool) -> Result<Vec<CString>> {
    let bytes = path.as_os_str().as_bytes();
    validate_total_path_length(bytes)?;
    if bytes.starts_with(b"/") {
        return Err(LocalFileError::InvalidPath("a relative path is required"));
    }
    parse_components(bytes, allow_empty)
}

fn parse_components(bytes: &[u8], allow_empty_path: bool) -> Result<Vec<CString>> {
    if bytes.is_empty() {
        return if allow_empty_path {
            Ok(Vec::new())
        } else {
            Err(LocalFileError::InvalidPath("an empty path is not allowed"))
        };
    }

    if bytes.split(|byte| *byte == b'/').count() > MAX_LOCAL_PATH_COMPONENTS {
        return Err(LocalFileError::InvalidPath(
            "path has more than 64 components",
        ));
    }

    bytes
        .split(|byte| *byte == b'/')
        .map(|component| {
            if component.is_empty() {
                return Err(LocalFileError::InvalidPath(
                    "empty path components are not allowed",
                ));
            }
            if component == b"." {
                return Err(LocalFileError::InvalidPath(
                    "dot path components are not allowed",
                ));
            }
            if component == b".." {
                return Err(LocalFileError::InvalidPath(
                    "parent path components are not allowed",
                ));
            }
            if component.len() > MAX_LOCAL_COMPONENT_BYTES {
                return Err(LocalFileError::InvalidPath(
                    "path component exceeds the 255-byte limit",
                ));
            }
            CString::new(component).map_err(|_| {
                LocalFileError::InvalidPath("NUL bytes are not allowed in local paths")
            })
        })
        .collect()
}

fn validate_total_path_length(bytes: &[u8]) -> Result<()> {
    if bytes.len() > MAX_LOCAL_PATH_BYTES {
        Err(LocalFileError::InvalidPath(
            "path exceeds the 4096-byte limit",
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::fs;
    use std::os::unix::fs::{PermissionsExt, symlink};
    use std::path::{Path, PathBuf};

    use super::*;

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new(label: &str) -> Self {
            // macOS exposes /var as a compatibility symlink. Resolve only the
            // synthetic test base so successful-path cases do not
            // intentionally trip the production symlink rejection.
            let test_base = std::env::temp_dir().canonicalize().unwrap();
            let path = test_base.join(format!(
                "lurkline-local-file-{label}-{}-{}",
                std::process::id(),
                TEMPORARY_SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn assert_directory_only_contains(directory: &Path, expected: &[&str]) {
        let mut names: Vec<_> = fs::read_dir(directory)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        names.sort();
        let mut expected: Vec<_> = expected.iter().map(OsStr::new).collect();
        expected.sort();
        assert_eq!(names, expected);
    }

    #[test]
    fn mcp_root_requires_absolute_path_and_relative_destination() {
        assert!(matches!(
            McpFileRoot::open(Path::new("relative")),
            Err(LocalFileError::InvalidPath(_))
        ));

        let root = TestDirectory::new("mcp-path-shapes");
        let capability = McpFileRoot::open(root.path()).unwrap();
        assert!(matches!(
            capability.prepare_download(Path::new("/absolute"), 10),
            Err(LocalFileError::InvalidPath(_))
        ));
    }

    #[test]
    fn rejects_empty_dot_parent_and_non_normal_components() {
        for path in ["", ".", "..", "a/../b", "a/./b", "a//b", "a/"] {
            assert!(
                matches!(
                    parse_relative(Path::new(path), false),
                    Err(LocalFileError::InvalidPath(_))
                ),
                "{path:?} should be rejected"
            );
        }
        assert!(parse_relative(Path::new("a/b"), false).is_ok());
        assert!(validate_cli_download_path(Path::new("a/b")).is_ok());
        assert!(validate_cli_download_path(Path::new("/a/b")).is_ok());
        assert!(validate_mcp_download_path(Path::new("a/b")).is_ok());
        assert!(validate_mcp_download_path(Path::new("/a/b")).is_err());

        let oversized_component = "x".repeat(MAX_LOCAL_COMPONENT_BYTES + 1);
        assert!(matches!(
            validate_cli_download_path(Path::new(&oversized_component)),
            Err(LocalFileError::InvalidPath(
                "path component exceeds the 255-byte limit"
            ))
        ));

        let too_many_components =
            std::iter::repeat_n("x", MAX_LOCAL_PATH_COMPONENTS + 1).collect::<Vec<_>>();
        assert!(matches!(
            validate_mcp_download_path(Path::new(&too_many_components.join("/"))),
            Err(LocalFileError::InvalidPath(
                "path has more than 64 components"
            ))
        ));

        let oversized_path = std::iter::repeat_n("x".repeat(255), 17)
            .collect::<Vec<_>>()
            .join("/");
        assert!(oversized_path.len() > MAX_LOCAL_PATH_BYTES);
        assert!(matches!(
            validate_cli_download_path(Path::new(&oversized_path)),
            Err(LocalFileError::InvalidPath(
                "path exceeds the 4096-byte limit"
            ))
        ));
    }

    #[test]
    fn descriptor_traversal_rejects_ancestor_and_leaf_symlinks() {
        let root = TestDirectory::new("symlink");
        let outside = TestDirectory::new("outside");
        fs::create_dir(root.path().join("real")).unwrap();
        symlink(outside.path(), root.path().join("ancestor-link")).unwrap();
        symlink(
            outside.path().join("target"),
            root.path().join("real").join("leaf-link"),
        )
        .unwrap();
        let capability = McpFileRoot::open(root.path()).unwrap();

        assert!(matches!(
            capability.prepare_download(Path::new("ancestor-link/output"), 10),
            Err(LocalFileError::Symlink | LocalFileError::NotDirectory)
        ));
        assert!(matches!(
            capability.prepare_download(Path::new("real/leaf-link"), 10),
            Err(LocalFileError::Symlink)
        ));
    }

    #[test]
    fn mcp_capability_remains_anchored_to_the_opened_root_descriptor() {
        let outer = TestDirectory::new("root-replacement");
        let configured = outer.path().join("configured");
        let opened = outer.path().join("opened");
        fs::create_dir(&configured).unwrap();
        let capability = McpFileRoot::open(&configured).unwrap();

        fs::rename(&configured, &opened).unwrap();
        fs::create_dir(&configured).unwrap();
        let mut download = capability
            .prepare_download(Path::new("output"), 10)
            .unwrap();
        download.write_chunk(b"safe").unwrap();
        download.commit().unwrap();

        assert_eq!(fs::read(opened.join("output")).unwrap(), b"safe");
        assert_directory_only_contains(&configured, &[]);
    }

    #[test]
    fn commit_detects_ancestor_replacement_and_cleans_the_temporary() {
        let root = TestDirectory::new("ancestor-replacement");
        fs::create_dir(root.path().join("parent")).unwrap();
        let capability = McpFileRoot::open(root.path()).unwrap();
        let mut download = capability
            .prepare_download(Path::new("parent/output"), 10)
            .unwrap();
        download.write_chunk(b"safe").unwrap();

        fs::rename(
            root.path().join("parent"),
            root.path().join("displaced-parent"),
        )
        .unwrap();
        fs::create_dir(root.path().join("parent")).unwrap();
        assert!(matches!(
            download.commit(),
            Err(LocalFileError::ParentChanged)
        ));
        assert_directory_only_contains(&root.path().join("parent"), &[]);
        assert_directory_only_contains(&root.path().join("displaced-parent"), &[]);
    }

    #[test]
    fn successful_commit_is_exact_mode_and_content() {
        let root = TestDirectory::new("success");
        let capability = McpFileRoot::open(root.path()).unwrap();
        let mut download = capability
            .prepare_download(Path::new("output"), 10)
            .unwrap();
        download.write_chunk(b"safe").unwrap();
        let result = download.commit().unwrap();

        assert_eq!(
            result,
            DownloadCommit {
                bytes_written: 4,
                durability: DownloadDurability::Synced
            }
        );
        let path = root.path().join("output");
        assert_eq!(fs::read(&path).unwrap(), b"safe");
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn direct_cli_absolute_path_is_descriptor_anchored() {
        let root = TestDirectory::new("cli-absolute");
        let path = root.path().join("output");
        let mut download = prepare_cli_download(&path, 10).unwrap();
        download.write_chunk(b"safe").unwrap();
        download.commit().unwrap();
        assert_eq!(fs::read(path).unwrap(), b"safe");
    }

    #[test]
    fn drop_after_partial_response_removes_temporary_and_output() {
        let root = TestDirectory::new("partial");
        let capability = McpFileRoot::open(root.path()).unwrap();
        {
            let mut download = capability
                .prepare_download(Path::new("output"), 10)
                .unwrap();
            download.write_chunk(b"part").unwrap();
        }
        assert_directory_only_contains(root.path(), &[]);
    }

    #[test]
    fn size_limit_failure_removes_temporary_and_output() {
        let root = TestDirectory::new("limit");
        let capability = McpFileRoot::open(root.path()).unwrap();
        {
            let mut download = capability.prepare_download(Path::new("output"), 4).unwrap();
            download.write_chunk(b"1234").unwrap();
            assert!(matches!(
                download.write_chunk(b"5"),
                Err(LocalFileError::SizeLimit { limit: 4 })
            ));
        }
        assert_directory_only_contains(root.path(), &[]);
    }

    #[test]
    fn existing_destination_is_untouched() {
        let root = TestDirectory::new("collision");
        fs::write(root.path().join("output"), b"existing").unwrap();
        let capability = McpFileRoot::open(root.path()).unwrap();

        assert!(matches!(
            capability.prepare_download(Path::new("output"), 10),
            Err(LocalFileError::DestinationExists)
        ));
        assert_eq!(fs::read(root.path().join("output")).unwrap(), b"existing");
        assert_directory_only_contains(root.path(), &["output"]);
    }

    #[test]
    fn no_replace_commit_wins_race_without_clobbering_destination() {
        let root = TestDirectory::new("no-replace");
        let capability = McpFileRoot::open(root.path()).unwrap();
        let mut download = capability
            .prepare_download(Path::new("output"), 10)
            .unwrap();
        download.write_chunk(b"new").unwrap();
        fs::write(root.path().join("output"), b"existing").unwrap();

        assert!(matches!(
            download.commit(),
            Err(LocalFileError::DestinationExists)
        ));
        assert_eq!(fs::read(root.path().join("output")).unwrap(), b"existing");
        assert_directory_only_contains(root.path(), &["output"]);
    }

    #[test]
    fn directory_sync_failure_is_committed_success_with_warning() {
        let root = TestDirectory::new("sync-warning");
        let capability = McpFileRoot::open(root.path()).unwrap();
        let mut download = capability
            .prepare_download(Path::new("output"), 10)
            .unwrap();
        download.write_chunk(b"safe").unwrap();
        download.inject_directory_sync_failure();

        assert_eq!(
            download.commit().unwrap(),
            DownloadCommit {
                bytes_written: 4,
                durability: DownloadDurability::DirectorySyncWarning
            }
        );
        assert_eq!(fs::read(root.path().join("output")).unwrap(), b"safe");
        assert_directory_only_contains(root.path(), &["output"]);
    }
}
