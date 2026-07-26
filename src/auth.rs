use std::{
    collections::BTreeMap,
    env,
    ffi::OsString,
    fmt, fs,
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use fs4::FileExt;
use keyring::v1::{Entry, Error as KeyringError};
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use crate::{
    config::{
        Config, CredentialBundle, Secret, credential_bundle_from_getter, validate_base_url,
        validate_identifier,
    },
    error::{Error, Result},
};

const KEYRING_SERVICE: &str = "me.smarzola.lurkline.slack-session";
const REGISTRY_VERSION: u8 = 1;
const CREDENTIAL_VERSION: u8 = 1;
const MAX_REGISTRY_BYTES: u64 = 1024 * 1024;
static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct ProfileName(String);

impl ProfileName {
    pub(crate) fn parse(raw: &str) -> Result<Self> {
        if raw.is_empty()
            || raw.len() > 64
            || !raw
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        {
            return Err(Error::invalid_input(
                "profile",
                "must contain 1 to 64 ASCII letters, digits, '.', '_', or '-'",
            ));
        }
        Ok(Self(raw.to_owned()))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ProfileName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProfileMetadata {
    pub workspace_url: String,
    pub team_id: String,
}

impl ProfileMetadata {
    pub(crate) fn from_bundle(bundle: &CredentialBundle) -> Self {
        Self {
            workspace_url: bundle.workspace_url(),
            team_id: bundle.team_id.clone(),
        }
    }

    fn validate(&self) -> Result<()> {
        let workspace =
            validate_base_url(&self.workspace_url).map_err(|_| Error::InvalidProfileRegistry)?;
        if workspace.origin().ascii_serialization() != self.workspace_url {
            return Err(Error::InvalidProfileRegistry);
        }
        validate_identifier("SLACK_TEAM_ID", &self.team_id)
            .map_err(|_| Error::InvalidProfileRegistry)
    }

    pub(crate) fn matches(&self, bundle: &CredentialBundle) -> bool {
        self.workspace_url == bundle.workspace_url() && self.team_id == bundle.team_id
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProfileRegistryState {
    version: u8,
    pub default_profile: Option<String>,
    pub profiles: BTreeMap<String, ProfileMetadata>,
}

impl Default for ProfileRegistryState {
    fn default() -> Self {
        Self {
            version: REGISTRY_VERSION,
            default_profile: None,
            profiles: BTreeMap::new(),
        }
    }
}

impl ProfileRegistryState {
    fn validate(&self) -> Result<()> {
        if self.version != REGISTRY_VERSION {
            return Err(Error::InvalidProfileRegistry);
        }
        for (name, metadata) in &self.profiles {
            ProfileName::parse(name).map_err(|_| Error::InvalidProfileRegistry)?;
            metadata.validate()?;
        }
        if self.profiles.is_empty() != self.default_profile.is_none() {
            return Err(Error::InvalidProfileRegistry);
        }
        if self
            .default_profile
            .as_ref()
            .is_some_and(|profile| !self.profiles.contains_key(profile))
        {
            return Err(Error::InvalidProfileRegistry);
        }
        Ok(())
    }

    pub(crate) fn selected_profile(
        &self,
        explicit: Option<&str>,
        environment: Option<&str>,
    ) -> Result<ProfileName> {
        if let Some(raw) = explicit.or(environment) {
            return ProfileName::parse(raw);
        }
        self.default_profile
            .as_deref()
            .ok_or(Error::MissingProfile)
            .and_then(ProfileName::parse)
    }

    pub(crate) fn register(&mut self, profile: &ProfileName, metadata: ProfileMetadata) {
        self.profiles.insert(profile.to_string(), metadata);
        if self.default_profile.is_none() {
            self.default_profile = Some(profile.to_string());
        }
    }

    pub(crate) fn remove(&mut self, profile: &ProfileName) -> bool {
        let removed = self.profiles.remove(profile.as_str()).is_some();
        if removed && self.default_profile.as_deref() == Some(profile.as_str()) {
            self.default_profile = self.profiles.keys().next().cloned();
        }
        removed
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ProfileRegistry {
    path: PathBuf,
}

impl ProfileRegistry {
    pub(crate) fn discover() -> Result<Self> {
        #[cfg(target_os = "macos")]
        let directory = macos_registry_directory(env::var_os("HOME"))?;
        #[cfg(target_os = "linux")]
        let directory =
            linux_registry_directory(env::var_os("HOME"), env::var_os("XDG_CONFIG_HOME"))?;
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        let directory = {
            return Err(Error::ProfileRegistryRead);
        };
        Ok(Self {
            path: directory.join("profiles.json"),
        })
    }

    #[cfg(test)]
    fn at(path: PathBuf) -> Self {
        Self { path }
    }

    fn lock_shared(&self) -> Result<File> {
        let file = self.open_lock_file()?;
        FileExt::lock_shared(&file).map_err(|_| Error::ProfileRegistryLock)?;
        Ok(file)
    }

    fn lock_exclusive(&self) -> Result<File> {
        let file = self.open_lock_file()?;
        FileExt::lock(&file).map_err(|_| Error::ProfileRegistryLock)?;
        Ok(file)
    }

    fn open_lock_file(&self) -> Result<File> {
        let directory = self.path.parent().ok_or(Error::ProfileRegistryLock)?;
        fs::create_dir_all(directory).map_err(|_| Error::ProfileRegistryLock)?;
        set_lock_directory_permissions(directory)?;
        let path = directory.join("profiles.lock");
        open_lock_file(&path).map_err(|_| Error::ProfileRegistryLock)
    }

    pub(crate) fn load(&self) -> Result<ProfileRegistryState> {
        let mut file = match File::open(&self.path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(ProfileRegistryState::default());
            }
            Err(_) => return Err(Error::ProfileRegistryRead),
        };
        if file
            .metadata()
            .map_err(|_| Error::ProfileRegistryRead)?
            .len()
            > MAX_REGISTRY_BYTES
        {
            return Err(Error::InvalidProfileRegistry);
        }
        let mut encoded = Vec::new();
        file.read_to_end(&mut encoded)
            .map_err(|_| Error::ProfileRegistryRead)?;
        if encoded.len() as u64 > MAX_REGISTRY_BYTES {
            return Err(Error::InvalidProfileRegistry);
        }
        let registry: ProfileRegistryState =
            serde_json::from_slice(&encoded).map_err(|_| Error::InvalidProfileRegistry)?;
        registry.validate()?;
        Ok(registry)
    }

    pub(crate) fn save(&self, registry: &ProfileRegistryState) -> Result<()> {
        registry.validate()?;
        let encoded =
            serde_json::to_vec_pretty(registry).map_err(|_| Error::ProfileRegistryWrite)?;
        let directory = self.path.parent().ok_or(Error::ProfileRegistryWrite)?;
        fs::create_dir_all(directory).map_err(|_| Error::ProfileRegistryWrite)?;
        set_directory_permissions(directory)?;

        let mut temporary_path = None;
        let mut temporary_file = None;
        for _ in 0..16 {
            let suffix = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = directory.join(format!(
                ".profiles.json.tmp-{}-{suffix}",
                std::process::id()
            ));
            match open_temporary(&path) {
                Ok(file) => {
                    temporary_path = Some(path);
                    temporary_file = Some(file);
                    break;
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(_) => return Err(Error::ProfileRegistryWrite),
            }
        }
        let path = temporary_path.ok_or(Error::ProfileRegistryWrite)?;
        let mut file = temporary_file.ok_or(Error::ProfileRegistryWrite)?;
        let result = (|| {
            file.write_all(&encoded)
                .map_err(|_| Error::ProfileRegistryWrite)?;
            file.sync_all().map_err(|_| Error::ProfileRegistryWrite)?;
            fs::rename(&path, &self.path).map_err(|_| Error::ProfileRegistryWrite)?;
            sync_directory(directory)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(path);
        }
        result
    }
}

fn nonempty(value: Option<OsString>) -> Option<OsString> {
    value.filter(|value| !value.is_empty())
}

#[cfg(target_os = "macos")]
fn macos_registry_directory(home: Option<OsString>) -> Result<PathBuf> {
    nonempty(home)
        .map(PathBuf::from)
        .map(|home| {
            home.join("Library")
                .join("Application Support")
                .join("lurkline")
        })
        .ok_or(Error::ProfileRegistryRead)
}

#[cfg(any(test, target_os = "linux"))]
fn linux_registry_directory(
    home: Option<OsString>,
    xdg_config_home: Option<OsString>,
) -> Result<PathBuf> {
    match nonempty(xdg_config_home) {
        Some(value) if Path::new(&value).is_absolute() => Ok(PathBuf::from(value).join("lurkline")),
        Some(_) => Err(Error::ProfileRegistryRead),
        None => nonempty(home)
            .map(PathBuf::from)
            .map(|home| home.join(".config").join("lurkline"))
            .ok_or(Error::ProfileRegistryRead),
    }
}

#[cfg(unix)]
fn open_temporary(path: &Path) -> std::io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;

    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
fn open_temporary(path: &Path) -> std::io::Result<File> {
    OpenOptions::new().write(true).create_new(true).open(path)
}

#[cfg(unix)]
fn open_lock_file(path: &Path) -> std::io::Result<File> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(path)?;
    file.set_permissions(fs::Permissions::from_mode(0o600))?;
    Ok(file)
}

#[cfg(not(unix))]
fn open_lock_file(path: &Path) -> std::io::Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
}

#[cfg(unix)]
fn set_directory_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|_| Error::ProfileRegistryWrite)
}

#[cfg(unix)]
fn set_lock_directory_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|_| Error::ProfileRegistryLock)
}

#[cfg(not(unix))]
fn set_lock_directory_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(not(unix))]
fn set_directory_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

fn sync_directory(path: &Path) -> Result<()> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| Error::ProfileRegistryWrite)
}

pub(crate) trait CredentialStore {
    fn get(&self, profile: &ProfileName) -> Result<Option<Zeroizing<Vec<u8>>>>;
    fn set(&self, profile: &ProfileName, secret: &[u8]) -> Result<()>;
    fn delete(&self, profile: &ProfileName) -> Result<bool>;
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct NativeCredentialStore;

impl NativeCredentialStore {
    fn entry(profile: &ProfileName) -> Result<Entry> {
        Entry::new(KEYRING_SERVICE, profile.as_str())
            .map_err(|error| map_keyring_error(profile, error))
    }
}

impl CredentialStore for NativeCredentialStore {
    fn get(&self, profile: &ProfileName) -> Result<Option<Zeroizing<Vec<u8>>>> {
        match Self::entry(profile)?.get_secret() {
            Ok(secret) => Ok(Some(Zeroizing::new(secret))),
            Err(KeyringError::NoEntry) => Ok(None),
            Err(error) => Err(map_keyring_error(profile, error)),
        }
    }

    fn set(&self, profile: &ProfileName, secret: &[u8]) -> Result<()> {
        Self::entry(profile)?
            .set_secret(secret)
            .map_err(|error| map_keyring_error(profile, error))
    }

    fn delete(&self, profile: &ProfileName) -> Result<bool> {
        match Self::entry(profile)?.delete_credential() {
            Ok(()) => Ok(true),
            Err(KeyringError::NoEntry) => Ok(false),
            Err(error) => Err(map_keyring_error(profile, error)),
        }
    }
}

fn map_keyring_error(profile: &ProfileName, error: KeyringError) -> Error {
    match error {
        KeyringError::NoDefaultStore
        | KeyringError::NoStorageAccess(_)
        | KeyringError::NotSupportedByStore(_) => Error::CredentialStoreUnavailable,
        KeyringError::BadEncoding(mut bytes) => {
            bytes.zeroize();
            Error::InvalidStoredCredential {
                profile: profile.to_string(),
            }
        }
        KeyringError::BadDataFormat(mut bytes, _) => {
            bytes.zeroize();
            Error::InvalidStoredCredential {
                profile: profile.to_string(),
            }
        }
        _ => Error::CredentialStore,
    }
}

#[derive(Serialize)]
struct StoredCredentialRef<'a> {
    version: u8,
    base_url: &'a str,
    team_id: &'a str,
    token: &'a str,
    cookie: &'a str,
}

#[derive(Deserialize, Zeroize, ZeroizeOnDrop)]
#[serde(deny_unknown_fields)]
struct StoredCredentialOwned {
    version: u8,
    base_url: String,
    team_id: String,
    token: String,
    cookie: String,
}

pub(crate) fn encode_bundle(bundle: &CredentialBundle) -> Result<Zeroizing<Vec<u8>>> {
    let stored = StoredCredentialRef {
        version: CREDENTIAL_VERSION,
        base_url: bundle.base_url.as_str(),
        team_id: &bundle.team_id,
        token: bundle.token(),
        cookie: bundle.cookie(),
    };
    serde_json::to_vec(&stored)
        .map(Zeroizing::new)
        .map_err(|_| Error::CredentialStore)
}

pub(crate) fn decode_bundle(profile: &ProfileName, encoded: &[u8]) -> Result<CredentialBundle> {
    let mut stored: StoredCredentialOwned =
        serde_json::from_slice(encoded).map_err(|_| Error::InvalidStoredCredential {
            profile: profile.to_string(),
        })?;
    if stored.version != CREDENTIAL_VERSION {
        return Err(Error::InvalidStoredCredential {
            profile: profile.to_string(),
        });
    }
    let parsed = (|| {
        let token = Secret::parse("SLACK_TOKEN", std::mem::take(&mut stored.token))?;
        let cookie = Secret::parse("SLACK_COOKIE", std::mem::take(&mut stored.cookie))?;
        CredentialBundle::parse_with_secrets(
            std::mem::take(&mut stored.base_url),
            std::mem::take(&mut stored.team_id),
            token,
            cookie,
        )
    })();
    parsed.map_err(|_| Error::InvalidStoredCredential {
        profile: profile.to_string(),
    })
}

trait RegistryStore {
    fn load_state(&self) -> Result<ProfileRegistryState>;
    fn save_state(&self, state: &ProfileRegistryState) -> Result<()>;
}

impl RegistryStore for ProfileRegistry {
    fn load_state(&self) -> Result<ProfileRegistryState> {
        self.load()
    }

    fn save_state(&self, state: &ProfileRegistryState) -> Result<()> {
        self.save(state)
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct AuthProfile {
    pub profile: String,
    pub workspace_url: String,
    pub team_id: String,
    pub default: bool,
}

#[derive(Debug, Serialize)]
pub(crate) struct AuthListReport {
    pub default_profile: Option<String>,
    pub profiles: Vec<AuthProfile>,
}

#[derive(Debug, Serialize)]
pub(crate) struct AuthStatusReport {
    pub profile: String,
    pub workspace_url: String,
    pub team_id: String,
    pub default: bool,
    pub credential_present: bool,
}

#[derive(Debug, Serialize)]
pub(crate) struct AuthImportReport {
    pub profile: String,
    pub workspace_url: String,
    pub team_id: String,
    pub default: bool,
    pub replaced_workspace: bool,
}

#[derive(Debug, Serialize)]
pub(crate) struct AuthRemoveReport {
    pub profile: String,
    pub removed: bool,
    pub default_profile: Option<String>,
}

pub(crate) fn list_profiles() -> Result<AuthListReport> {
    let registry = ProfileRegistry::discover()?;
    let _lock = registry.lock_shared()?;
    list_profiles_with(&registry)
}

fn list_profiles_with(registry: &impl RegistryStore) -> Result<AuthListReport> {
    let state = registry.load_state()?;
    let profiles = state
        .profiles
        .iter()
        .map(|(profile, metadata)| AuthProfile {
            profile: profile.clone(),
            workspace_url: metadata.workspace_url.clone(),
            team_id: metadata.team_id.clone(),
            default: state.default_profile.as_deref() == Some(profile),
        })
        .collect();
    Ok(AuthListReport {
        default_profile: state.default_profile,
        profiles,
    })
}

pub(crate) fn profile_status(explicit_profile: Option<&str>) -> Result<AuthStatusReport> {
    let registry = ProfileRegistry::discover()?;
    let _lock = registry.lock_shared()?;
    profile_status_with(
        explicit_profile,
        env::var("LURKLINE_PROFILE").ok().as_deref(),
        &registry,
        &NativeCredentialStore,
    )
}

fn profile_status_with(
    explicit_profile: Option<&str>,
    environment_profile: Option<&str>,
    registry: &impl RegistryStore,
    store: &impl CredentialStore,
) -> Result<AuthStatusReport> {
    let state = registry.load_state()?;
    let profile = state.selected_profile(explicit_profile, environment_profile)?;
    let metadata = state
        .profiles
        .get(profile.as_str())
        .ok_or_else(|| Error::ProfileNotFound {
            profile: profile.to_string(),
        })?;
    let credential_present = match store.get(&profile)? {
        Some(encoded) => {
            let bundle = decode_bundle(&profile, &encoded)?;
            if !metadata.matches(&bundle) {
                return Err(Error::CredentialProfileMismatch {
                    profile: profile.to_string(),
                });
            }
            true
        }
        None => false,
    };
    Ok(AuthStatusReport {
        profile: profile.to_string(),
        workspace_url: metadata.workspace_url.clone(),
        team_id: metadata.team_id.clone(),
        default: state.default_profile.as_deref() == Some(profile.as_str()),
        credential_present,
    })
}

pub(crate) fn store_profile(
    profile: &ProfileName,
    bundle: CredentialBundle,
    replace_workspace: bool,
) -> Result<AuthImportReport> {
    let registry = ProfileRegistry::discover()?;
    store_profile_locked(
        profile,
        bundle,
        replace_workspace,
        &registry,
        &NativeCredentialStore,
    )
}

fn store_profile_locked(
    profile: &ProfileName,
    bundle: CredentialBundle,
    replace_workspace: bool,
    registry: &ProfileRegistry,
    store: &impl CredentialStore,
) -> Result<AuthImportReport> {
    let _lock = registry.lock_exclusive()?;
    store_profile_with(profile, bundle, replace_workspace, registry, store)
}

fn store_profile_with(
    profile: &ProfileName,
    bundle: CredentialBundle,
    replace_workspace: bool,
    registry: &impl RegistryStore,
    store: &impl CredentialStore,
) -> Result<AuthImportReport> {
    let mut state = registry.load_state()?;
    let metadata = ProfileMetadata::from_bundle(&bundle);
    let current = state.profiles.get(profile.as_str());
    let replaced_workspace = current.is_some_and(|existing| existing != &metadata);
    if replaced_workspace && !replace_workspace {
        return Err(Error::ProfileWorkspaceMismatch {
            profile: profile.to_string(),
        });
    }

    let encoded = encode_bundle(&bundle)?;
    if current.is_some() && !replaced_workspace {
        store.set(profile, &encoded)?;
        return Ok(AuthImportReport {
            profile: profile.to_string(),
            workspace_url: metadata.workspace_url,
            team_id: metadata.team_id,
            default: state.default_profile.as_deref() == Some(profile.as_str()),
            replaced_workspace: false,
        });
    }

    let previous = store.get(profile)?;
    store.set(profile, &encoded)?;
    state.register(profile, metadata.clone());
    if let Err(error) = registry.save_state(&state) {
        let rollback = match previous {
            Some(previous) => store.set(profile, &previous),
            None => store.delete(profile).map(|_| ()),
        };
        if rollback.is_err() {
            return Err(Error::CredentialReconciliation {
                profile: profile.to_string(),
            });
        }
        return Err(error);
    }

    Ok(AuthImportReport {
        profile: profile.to_string(),
        workspace_url: metadata.workspace_url,
        team_id: metadata.team_id,
        default: state.default_profile.as_deref() == Some(profile.as_str()),
        replaced_workspace,
    })
}

pub(crate) fn remove_profile(explicit_profile: Option<&str>) -> Result<AuthRemoveReport> {
    let registry = ProfileRegistry::discover()?;
    remove_profile_locked(
        explicit_profile,
        env::var("LURKLINE_PROFILE").ok().as_deref(),
        &registry,
        &NativeCredentialStore,
    )
}

fn remove_profile_locked(
    explicit_profile: Option<&str>,
    environment_profile: Option<&str>,
    registry: &ProfileRegistry,
    store: &impl CredentialStore,
) -> Result<AuthRemoveReport> {
    let _lock = registry.lock_exclusive()?;
    remove_profile_with(explicit_profile, environment_profile, registry, store)
}

fn remove_profile_with(
    explicit_profile: Option<&str>,
    environment_profile: Option<&str>,
    registry: &impl RegistryStore,
    store: &impl CredentialStore,
) -> Result<AuthRemoveReport> {
    let mut state = registry.load_state()?;
    let profile = state.selected_profile(explicit_profile, environment_profile)?;
    if !state.profiles.contains_key(profile.as_str()) {
        return Err(Error::ProfileNotFound {
            profile: profile.to_string(),
        });
    }
    store.delete(&profile)?;
    state.remove(&profile);
    registry.save_state(&state)?;
    Ok(AuthRemoveReport {
        profile: profile.to_string(),
        removed: true,
        default_profile: state.default_profile,
    })
}

pub(crate) fn resolve_config(explicit_profile: Option<&str>) -> Result<Config> {
    let mut get = environment_value;
    if let Some(bundle) = credential_bundle_from_getter(&mut get)? {
        return Config::from_bundle_getter(bundle, get);
    }
    let registry = ProfileRegistry::discover()?;
    resolve_stored_config(explicit_profile, get, &registry, &NativeCredentialStore)
}

fn environment_value(name: &str) -> Option<String> {
    env::var(name).ok()
}

#[cfg(test)]
fn resolve_config_with(
    explicit_profile: Option<&str>,
    mut get: impl FnMut(&str) -> Option<String>,
    registry: &ProfileRegistry,
    store: &impl CredentialStore,
) -> Result<Config> {
    if let Some(bundle) = credential_bundle_from_getter(&mut get)? {
        return Config::from_bundle_getter(bundle, get);
    }
    resolve_stored_config(explicit_profile, get, registry, store)
}

fn resolve_stored_config(
    explicit_profile: Option<&str>,
    mut get: impl FnMut(&str) -> Option<String>,
    registry: &ProfileRegistry,
    store: &impl CredentialStore,
) -> Result<Config> {
    let _lock = registry.lock_shared()?;
    let environment_profile = get("LURKLINE_PROFILE");
    let state = registry.load()?;
    let profile = state.selected_profile(explicit_profile, environment_profile.as_deref())?;
    let metadata = state
        .profiles
        .get(profile.as_str())
        .ok_or_else(|| Error::ProfileNotFound {
            profile: profile.to_string(),
        })?;
    let encoded = store.get(&profile)?.ok_or_else(|| Error::ProfileNotFound {
        profile: profile.to_string(),
    })?;
    let bundle = decode_bundle(&profile, &encoded)?;
    if !metadata.matches(&bundle) {
        return Err(Error::CredentialProfileMismatch {
            profile: profile.to_string(),
        });
    }
    Config::from_bundle_getter(bundle, get)
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{HashMap, VecDeque},
        sync::{
            Arc, Barrier, Mutex,
            atomic::{AtomicUsize, Ordering},
        },
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new(label: &str) -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path =
                env::temp_dir().join(format!("lurkline-{label}-{}-{nonce}", std::process::id()));
            Self(path)
        }

        fn registry(&self) -> ProfileRegistry {
            ProfileRegistry::at(self.0.join("profiles.json"))
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[derive(Default)]
    struct MemoryStore {
        values: Mutex<BTreeMap<String, Vec<u8>>>,
        reads: AtomicUsize,
        sets: AtomicUsize,
        deletes: AtomicUsize,
        set_failures: Mutex<VecDeque<bool>>,
        delete_failures: Mutex<VecDeque<bool>>,
    }

    impl MemoryStore {
        fn insert(&self, profile: &ProfileName, bundle: &CredentialBundle) {
            self.values
                .lock()
                .unwrap()
                .insert(profile.to_string(), encode_bundle(bundle).unwrap().to_vec());
        }

        fn fail_next_set(&self) {
            self.set_failures.lock().unwrap().push_back(true);
        }

        fn fail_next_delete(&self) {
            self.delete_failures.lock().unwrap().push_back(true);
        }

        fn queue_set_results(&self, failures: impl IntoIterator<Item = bool>) {
            self.set_failures.lock().unwrap().extend(failures);
        }

        fn decoded(&self, profile: &ProfileName) -> Option<CredentialBundle> {
            self.values
                .lock()
                .unwrap()
                .get(profile.as_str())
                .map(|encoded| decode_bundle(profile, encoded).unwrap())
        }
    }

    impl CredentialStore for MemoryStore {
        fn get(&self, profile: &ProfileName) -> Result<Option<Zeroizing<Vec<u8>>>> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            Ok(self
                .values
                .lock()
                .unwrap()
                .get(profile.as_str())
                .cloned()
                .map(Zeroizing::new))
        }

        fn set(&self, profile: &ProfileName, secret: &[u8]) -> Result<()> {
            self.sets.fetch_add(1, Ordering::SeqCst);
            if self
                .set_failures
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(false)
            {
                return Err(Error::CredentialStore);
            }
            self.values
                .lock()
                .unwrap()
                .insert(profile.to_string(), secret.to_vec());
            Ok(())
        }

        fn delete(&self, profile: &ProfileName) -> Result<bool> {
            self.deletes.fetch_add(1, Ordering::SeqCst);
            if self
                .delete_failures
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(false)
            {
                return Err(Error::CredentialStore);
            }
            Ok(self
                .values
                .lock()
                .unwrap()
                .remove(profile.as_str())
                .is_some())
        }
    }

    #[derive(Default)]
    struct MemoryRegistry {
        state: Mutex<ProfileRegistryState>,
        saves: AtomicUsize,
        save_failures: Mutex<VecDeque<bool>>,
    }

    impl MemoryRegistry {
        fn fail_next_save(&self) {
            self.save_failures.lock().unwrap().push_back(true);
        }
    }

    impl RegistryStore for MemoryRegistry {
        fn load_state(&self) -> Result<ProfileRegistryState> {
            Ok(self.state.lock().unwrap().clone())
        }

        fn save_state(&self, state: &ProfileRegistryState) -> Result<()> {
            self.saves.fetch_add(1, Ordering::SeqCst);
            if self
                .save_failures
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(false)
            {
                return Err(Error::ProfileRegistryWrite);
            }
            *self.state.lock().unwrap() = state.clone();
            Ok(())
        }
    }

    fn bundle(workspace: &str, team_id: &str, token: &str) -> CredentialBundle {
        CredentialBundle::parse(
            format!("https://{workspace}.slack.com"),
            team_id.into(),
            token.into(),
            "d=xoxd-test-cookie; b=test".into(),
        )
        .unwrap()
    }

    fn environment(bundle: &CredentialBundle) -> HashMap<&'static str, String> {
        HashMap::from([
            ("SLACK_BASE_URL", bundle.workspace_url()),
            ("SLACK_TEAM_ID", bundle.team_id.clone()),
            ("SLACK_TOKEN", bundle.token().into()),
            ("SLACK_COOKIE", bundle.cookie().into()),
        ])
    }

    #[test]
    fn profile_names_are_bounded_and_safe_for_keyring_accounts() {
        for valid in ["default", "sferait-ws", "work_2", "team.eu"] {
            assert_eq!(ProfileName::parse(valid).unwrap().as_str(), valid);
        }
        for invalid in ["", "has space", "../escape", "line\nbreak", &"a".repeat(65)] {
            assert!(ProfileName::parse(invalid).is_err(), "{invalid:?}");
        }
    }

    #[test]
    fn linux_config_path_prefers_absolute_xdg_without_requiring_home() {
        assert_eq!(
            linux_registry_directory(None, Some(OsString::from("/tmp/xdg"))).unwrap(),
            PathBuf::from("/tmp/xdg/lurkline")
        );
        assert_eq!(
            linux_registry_directory(Some(OsString::from("/home/test")), None).unwrap(),
            PathBuf::from("/home/test/.config/lurkline")
        );
        assert!(linux_registry_directory(None, Some(OsString::from("relative"))).is_err());
        assert!(linux_registry_directory(None, None).is_err());
    }

    #[test]
    fn registry_default_lifecycle_is_deterministic() {
        let directory = TestDirectory::new("registry-lifecycle");
        let registry = directory.registry();
        let alpha = ProfileName::parse("alpha").unwrap();
        let beta = ProfileName::parse("beta").unwrap();
        let gamma = ProfileName::parse("gamma").unwrap();
        let mut state = ProfileRegistryState::default();
        state.register(
            &beta,
            ProfileMetadata::from_bundle(&bundle("beta", "TBETA", "token")),
        );
        state.register(
            &gamma,
            ProfileMetadata::from_bundle(&bundle("gamma", "TGAMMA", "token")),
        );
        state.register(
            &alpha,
            ProfileMetadata::from_bundle(&bundle("alpha", "TALPHA", "token")),
        );
        assert_eq!(state.default_profile.as_deref(), Some("beta"));
        registry.save(&state).unwrap();

        let mut loaded = registry.load().unwrap();
        assert_eq!(loaded, state);
        assert!(loaded.remove(&beta));
        assert_eq!(loaded.default_profile.as_deref(), Some("alpha"));
        assert!(loaded.remove(&alpha));
        assert_eq!(loaded.default_profile.as_deref(), Some("gamma"));
        assert!(loaded.remove(&gamma));
        assert_eq!(loaded.default_profile, None);
    }

    #[test]
    fn registry_rejects_corruption_without_overwriting_it() {
        let directory = TestDirectory::new("registry-corruption");
        fs::create_dir_all(directory.0.clone()).unwrap();
        let registry = directory.registry();
        fs::write(&registry.path, b"{not-json").unwrap();
        assert!(matches!(
            registry.load(),
            Err(Error::InvalidProfileRegistry)
        ));
        assert_eq!(fs::read(&registry.path).unwrap(), b"{not-json");

        let stale = serde_json::json!({
            "version": 1,
            "default_profile": "missing",
            "profiles": {}
        });
        fs::write(&registry.path, serde_json::to_vec(&stale).unwrap()).unwrap();
        assert!(matches!(
            registry.load(),
            Err(Error::InvalidProfileRegistry)
        ));

        let invalid_name = serde_json::json!({
            "version": 1,
            "default_profile": null,
            "profiles": {
                "../escape": {
                    "workspace_url": "https://example.slack.com",
                    "team_id": "T123"
                }
            }
        });
        fs::write(&registry.path, serde_json::to_vec(&invalid_name).unwrap()).unwrap();
        assert!(matches!(
            registry.load(),
            Err(Error::InvalidProfileRegistry)
        ));

        let missing_default = serde_json::json!({
            "version": 1,
            "default_profile": null,
            "profiles": {
                "work": {
                    "workspace_url": "https://example.slack.com",
                    "team_id": "T123"
                }
            }
        });
        fs::write(
            &registry.path,
            serde_json::to_vec(&missing_default).unwrap(),
        )
        .unwrap();
        assert!(matches!(
            registry.load(),
            Err(Error::InvalidProfileRegistry)
        ));
    }

    #[test]
    fn empty_registry_has_no_implicit_profile() {
        let state = ProfileRegistryState::default();
        assert!(matches!(
            state.selected_profile(None, None),
            Err(Error::MissingProfile)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn registry_is_owner_only_on_unix() {
        use std::os::unix::fs::PermissionsExt;

        let directory = TestDirectory::new("registry-mode");
        let registry = directory.registry();
        registry.save(&ProfileRegistryState::default()).unwrap();
        assert_eq!(
            fs::metadata(&registry.path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(registry.path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }

    #[test]
    fn complete_environment_overrides_profiles_without_store_access() {
        let directory = TestDirectory::new("environment");
        let registry = directory.registry();
        fs::create_dir_all(directory.0.clone()).unwrap();
        fs::write(&registry.path, b"invalid").unwrap();
        let store = MemoryStore::default();
        let bundle = bundle("environment", "TENV", "legacy-browser-token");
        let values = environment(&bundle);

        let config = resolve_config_with(
            Some("ignored"),
            |name| values.get(name).cloned(),
            &registry,
            &store,
        )
        .unwrap();
        assert_eq!(config.team_id, "TENV");
        assert_eq!(config.token.expose(), "legacy-browser-token");
        assert_eq!(store.reads.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn partial_environment_fails_before_registry_or_store_access() {
        let directory = TestDirectory::new("partial-environment");
        let registry = directory.registry();
        fs::create_dir_all(directory.0.clone()).unwrap();
        fs::write(&registry.path, b"invalid").unwrap();
        let store = MemoryStore::default();
        let values = HashMap::from([("SLACK_TOKEN", "legacy-token".to_owned())]);

        let error = resolve_config_with(
            Some("ignored"),
            |name| values.get(name).cloned(),
            &registry,
            &store,
        )
        .unwrap_err();
        assert!(matches!(error, Error::MissingConfig("SLACK_BASE_URL")));
        assert_eq!(store.reads.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn explicit_then_environment_then_default_profile_precedence() {
        let directory = TestDirectory::new("precedence");
        let registry = directory.registry();
        let store = MemoryStore::default();
        let mut state = ProfileRegistryState::default();
        for (name, workspace, team) in [
            ("default", "default", "TDEFAULT"),
            ("environment", "environment", "TENV"),
            ("explicit", "explicit", "TEXPLICIT"),
        ] {
            let profile = ProfileName::parse(name).unwrap();
            let bundle = bundle(workspace, team, "stored-token");
            state.register(&profile, ProfileMetadata::from_bundle(&bundle));
            store.insert(&profile, &bundle);
        }
        registry.save(&state).unwrap();

        let explicit = resolve_config_with(
            Some("explicit"),
            |name| (name == "LURKLINE_PROFILE").then(|| "environment".into()),
            &registry,
            &store,
        )
        .unwrap();
        assert_eq!(explicit.team_id, "TEXPLICIT");

        let environment = resolve_config_with(
            None,
            |name| (name == "LURKLINE_PROFILE").then(|| "environment".into()),
            &registry,
            &store,
        )
        .unwrap();
        assert_eq!(environment.team_id, "TENV");

        let default =
            resolve_config_with(None, |_| None, &registry, &store).expect("default profile");
        assert_eq!(default.team_id, "TDEFAULT");
    }

    #[test]
    fn resolver_rejects_missing_or_mismatched_stored_credentials() {
        let directory = TestDirectory::new("stored-mismatch");
        let registry = directory.registry();
        let store = MemoryStore::default();
        let profile = ProfileName::parse("work").unwrap();
        let registered = bundle("registered", "TREGISTERED", "stored-token");
        let mut state = ProfileRegistryState::default();
        state.register(&profile, ProfileMetadata::from_bundle(&registered));
        registry.save(&state).unwrap();

        let missing = resolve_config_with(None, |_| None, &registry, &store).unwrap_err();
        assert!(matches!(missing, Error::ProfileNotFound { .. }));

        store.insert(&profile, &bundle("other", "TOTHER", "stored-token"));
        let mismatch = resolve_config_with(None, |_| None, &registry, &store).unwrap_err();
        assert!(matches!(mismatch, Error::CredentialProfileMismatch { .. }));
    }

    #[test]
    fn profile_management_preserves_defaults_and_requires_explicit_replacement() {
        let registry = MemoryRegistry::default();
        let store = MemoryStore::default();
        let beta = ProfileName::parse("beta").unwrap();
        let alpha = ProfileName::parse("alpha").unwrap();

        let first = store_profile_with(
            &beta,
            bundle("beta", "TBETA", "xoxc-first"),
            false,
            &registry,
            &store,
        )
        .unwrap();
        assert!(first.default);
        assert_eq!(
            serde_json::to_value(&first).unwrap(),
            serde_json::json!({
                "profile": "beta",
                "workspace_url": "https://beta.slack.com",
                "team_id": "TBETA",
                "default": true,
                "replaced_workspace": false
            })
        );
        let second = store_profile_with(
            &alpha,
            bundle("alpha", "TALPHA", "xoxc-second"),
            false,
            &registry,
            &store,
        )
        .unwrap();
        assert!(!second.default);

        let list = list_profiles_with(&registry).unwrap();
        assert_eq!(list.default_profile.as_deref(), Some("beta"));
        assert_eq!(
            list.profiles
                .iter()
                .map(|profile| profile.profile.as_str())
                .collect::<Vec<_>>(),
            ["alpha", "beta"]
        );
        assert_eq!(
            serde_json::to_value(&list).unwrap(),
            serde_json::json!({
                "default_profile": "beta",
                "profiles": [
                    {
                        "profile": "alpha",
                        "workspace_url": "https://alpha.slack.com",
                        "team_id": "TALPHA",
                        "default": false
                    },
                    {
                        "profile": "beta",
                        "workspace_url": "https://beta.slack.com",
                        "team_id": "TBETA",
                        "default": true
                    }
                ]
            })
        );
        let status = profile_status_with(Some("alpha"), None, &registry, &store).unwrap();
        assert!(status.credential_present);
        assert!(!status.default);
        assert_eq!(
            serde_json::to_value(&status).unwrap(),
            serde_json::json!({
                "profile": "alpha",
                "workspace_url": "https://alpha.slack.com",
                "team_id": "TALPHA",
                "default": false,
                "credential_present": true
            })
        );
        let rendered = serde_json::to_string(&status).unwrap();
        assert!(!rendered.contains("xoxc-"));
        assert!(!rendered.contains("xoxd-"));

        let saves_before_refresh = registry.saves.load(Ordering::SeqCst);
        store_profile_with(
            &beta,
            bundle("beta", "TBETA", "xoxc-refreshed"),
            false,
            &registry,
            &store,
        )
        .unwrap();
        assert_eq!(
            registry.saves.load(Ordering::SeqCst),
            saves_before_refresh,
            "same-workspace refresh must update only keyring"
        );
        assert_eq!(store.decoded(&beta).unwrap().token(), "xoxc-refreshed");

        let mismatch = store_profile_with(
            &beta,
            bundle("other", "TOTHER", "xoxc-other"),
            false,
            &registry,
            &store,
        )
        .unwrap_err();
        assert!(matches!(mismatch, Error::ProfileWorkspaceMismatch { .. }));
        assert_eq!(store.decoded(&beta).unwrap().team_id, "TBETA");

        let replaced = store_profile_with(
            &beta,
            bundle("other", "TOTHER", "xoxc-other"),
            true,
            &registry,
            &store,
        )
        .unwrap();
        assert!(replaced.replaced_workspace);
        assert!(replaced.default);
        assert_eq!(store.decoded(&beta).unwrap().team_id, "TOTHER");
    }

    #[test]
    fn concurrent_profile_imports_preserve_both_registry_and_keyring_entries() {
        let directory = TestDirectory::new("concurrent-imports");
        let registry = directory.registry();
        let store = Arc::new(MemoryStore::default());
        let barrier = Arc::new(Barrier::new(3));

        let mut threads = Vec::new();
        for (name, workspace, team) in [("alpha", "alpha", "TALPHA"), ("beta", "beta", "TBETA")] {
            let registry = registry.clone();
            let store = Arc::clone(&store);
            let barrier = Arc::clone(&barrier);
            threads.push(std::thread::spawn(move || {
                let profile = ProfileName::parse(name).unwrap();
                barrier.wait();
                store_profile_locked(
                    &profile,
                    bundle(workspace, team, "xoxc-concurrent"),
                    false,
                    &registry,
                    store.as_ref(),
                )
                .unwrap();
            }));
        }
        barrier.wait();
        for thread in threads {
            thread.join().unwrap();
        }

        let state = registry.load().unwrap();
        assert_eq!(
            state
                .profiles
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["alpha", "beta"]
        );
        assert!(
            state
                .default_profile
                .as_ref()
                .is_some_and(|profile| { profile == "alpha" || profile == "beta" })
        );
        for profile in ["alpha", "beta"] {
            assert!(
                store
                    .decoded(&ProfileName::parse(profile).unwrap())
                    .is_some()
            );
        }
    }

    #[test]
    fn concurrent_profile_import_and_removal_leave_cross_store_state_consistent() {
        let directory = TestDirectory::new("concurrent-import-remove");
        let registry = directory.registry();
        let store = Arc::new(MemoryStore::default());
        for (name, workspace, team) in [("keep", "keep", "TKEEP"), ("remove", "remove", "TREMOVE")]
        {
            store_profile_locked(
                &ProfileName::parse(name).unwrap(),
                bundle(workspace, team, "xoxc-initial"),
                false,
                &registry,
                store.as_ref(),
            )
            .unwrap();
        }

        let barrier = Arc::new(Barrier::new(3));
        let import_thread = {
            let registry = registry.clone();
            let store = Arc::clone(&store);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                store_profile_locked(
                    &ProfileName::parse("new").unwrap(),
                    bundle("new", "TNEW", "xoxc-new"),
                    false,
                    &registry,
                    store.as_ref(),
                )
                .unwrap();
            })
        };
        let remove_thread = {
            let registry = registry.clone();
            let store = Arc::clone(&store);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                remove_profile_locked(Some("remove"), None, &registry, store.as_ref()).unwrap();
            })
        };
        barrier.wait();
        import_thread.join().unwrap();
        remove_thread.join().unwrap();

        let state = registry.load().unwrap();
        assert_eq!(
            state
                .profiles
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["keep", "new"]
        );
        assert_eq!(state.default_profile.as_deref(), Some("keep"));
        assert!(
            store
                .decoded(&ProfileName::parse("keep").unwrap())
                .is_some()
        );
        assert!(store.decoded(&ProfileName::parse("new").unwrap()).is_some());
        assert!(
            store
                .decoded(&ProfileName::parse("remove").unwrap())
                .is_none()
        );
    }

    #[test]
    fn profile_store_rolls_back_every_failure_boundary() {
        let profile = ProfileName::parse("work").unwrap();

        let registry = MemoryRegistry::default();
        let store = MemoryStore::default();
        store.fail_next_set();
        let error = store_profile_with(
            &profile,
            bundle("example", "T123", "xoxc-test"),
            false,
            &registry,
            &store,
        )
        .unwrap_err();
        assert!(matches!(error, Error::CredentialStore));
        assert!(registry.load_state().unwrap().profiles.is_empty());
        assert!(store.decoded(&profile).is_none());

        let registry = MemoryRegistry::default();
        let store = MemoryStore::default();
        registry.fail_next_save();
        let error = store_profile_with(
            &profile,
            bundle("example", "T123", "xoxc-test"),
            false,
            &registry,
            &store,
        )
        .unwrap_err();
        assert!(matches!(error, Error::ProfileRegistryWrite));
        assert!(registry.load_state().unwrap().profiles.is_empty());
        assert!(store.decoded(&profile).is_none());

        let registry = MemoryRegistry::default();
        let store = MemoryStore::default();
        store_profile_with(
            &profile,
            bundle("original", "TORIGINAL", "xoxc-original"),
            false,
            &registry,
            &store,
        )
        .unwrap();
        registry.fail_next_save();
        let error = store_profile_with(
            &profile,
            bundle("replacement", "TREPLACE", "xoxc-replacement"),
            true,
            &registry,
            &store,
        )
        .unwrap_err();
        assert!(matches!(error, Error::ProfileRegistryWrite));
        let restored = store.decoded(&profile).unwrap();
        assert_eq!(restored.team_id, "TORIGINAL");
        assert_eq!(restored.token(), "xoxc-original");

        registry.fail_next_save();
        store.queue_set_results([false, true]);
        let error = store_profile_with(
            &profile,
            bundle("replacement", "TREPLACE", "xoxc-replacement"),
            true,
            &registry,
            &store,
        )
        .unwrap_err();
        assert!(matches!(error, Error::CredentialReconciliation { .. }));
        assert_eq!(store.decoded(&profile).unwrap().team_id, "TREPLACE");

        let registry = MemoryRegistry::default();
        let store = MemoryStore::default();
        registry.fail_next_save();
        store.fail_next_delete();
        let error = store_profile_with(
            &profile,
            bundle("example", "T123", "xoxc-test"),
            false,
            &registry,
            &store,
        )
        .unwrap_err();
        assert!(matches!(error, Error::CredentialReconciliation { .. }));
        assert!(store.decoded(&profile).is_some());
    }

    #[test]
    fn profile_removal_is_retryable_and_reselects_default() {
        let registry = MemoryRegistry::default();
        let store = MemoryStore::default();
        let beta = ProfileName::parse("beta").unwrap();
        let alpha = ProfileName::parse("alpha").unwrap();
        store_profile_with(
            &beta,
            bundle("beta", "TBETA", "xoxc-beta"),
            false,
            &registry,
            &store,
        )
        .unwrap();
        store_profile_with(
            &alpha,
            bundle("alpha", "TALPHA", "xoxc-alpha"),
            false,
            &registry,
            &store,
        )
        .unwrap();

        store.fail_next_delete();
        let error = remove_profile_with(Some("beta"), None, &registry, &store).unwrap_err();
        assert!(matches!(error, Error::CredentialStore));
        assert_eq!(
            registry.load_state().unwrap().default_profile.as_deref(),
            Some("beta")
        );
        assert!(store.decoded(&beta).is_some());

        registry.fail_next_save();
        let error = remove_profile_with(Some("beta"), None, &registry, &store).unwrap_err();
        assert!(matches!(error, Error::ProfileRegistryWrite));
        assert!(store.decoded(&beta).is_none());
        assert!(registry.load_state().unwrap().profiles.contains_key("beta"));

        let retry = remove_profile_with(Some("beta"), None, &registry, &store).unwrap();
        assert!(retry.removed);
        assert_eq!(retry.default_profile.as_deref(), Some("alpha"));
        assert_eq!(
            serde_json::to_value(&retry).unwrap(),
            serde_json::json!({
                "profile": "beta",
                "removed": true,
                "default_profile": "alpha"
            })
        );
        let final_remove = remove_profile_with(Some("alpha"), None, &registry, &store).unwrap();
        assert_eq!(final_remove.default_profile, None);
        assert!(registry.load_state().unwrap().profiles.is_empty());
    }

    #[test]
    fn stored_bundle_round_trip_redacts_and_accepts_legacy_token_shapes() {
        let profile = ProfileName::parse("work").unwrap();
        let original = bundle("example", "T123", "legacy-browser-token");
        let encoded = encode_bundle(&original).unwrap();
        let decoded = decode_bundle(&profile, &encoded).unwrap();
        assert_eq!(decoded.workspace_url(), "https://example.slack.com");
        assert_eq!(decoded.team_id, "T123");
        assert_eq!(decoded.token(), "legacy-browser-token");
        let rendered = format!("{decoded:?}");
        assert!(!rendered.contains("legacy-browser-token"));
        assert!(!rendered.contains("xoxd-test-cookie"));
        assert_eq!(rendered.matches("[REDACTED]").count(), 2);
    }

    #[test]
    fn keyring_errors_are_secret_safe_and_actionable() {
        let profile = ProfileName::parse("work").unwrap();
        let invalid = map_keyring_error(
            &profile,
            KeyringError::BadEncoding(b"xoxc-should-never-render".to_vec()),
        );
        assert!(matches!(invalid, Error::InvalidStoredCredential { .. }));
        assert!(!invalid.to_string().contains("xoxc-should-never-render"));

        let malformed = map_keyring_error(
            &profile,
            KeyringError::BadDataFormat(
                b"d=xoxd-should-never-render".to_vec(),
                Box::new(std::io::Error::other("synthetic format error")),
            ),
        );
        assert!(matches!(malformed, Error::InvalidStoredCredential { .. }));
        assert!(!malformed.to_string().contains("xoxd-should-never-render"));

        let unavailable = map_keyring_error(&profile, KeyringError::NoDefaultStore);
        assert!(matches!(unavailable, Error::CredentialStoreUnavailable));
        assert!(unavailable.to_string().contains("SLACK_*"));

        let generic = map_keyring_error(
            &profile,
            KeyringError::Invalid("field".into(), "xoxc-hidden-detail".into()),
        );
        assert!(matches!(generic, Error::CredentialStore));
        assert!(generic.to_string().contains("unlock or configure"));
        assert!(!generic.to_string().contains("hidden-detail"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "writes one synthetic macOS Keychain entry and removes it immediately"]
    fn native_keyring_round_trip() {
        struct Cleanup(ProfileName);

        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = NativeCredentialStore.delete(&self.0);
            }
        }

        let profile = ProfileName::parse(&format!("native-smoke-{}", std::process::id())).unwrap();
        let _cleanup = Cleanup(profile.clone());
        let store = NativeCredentialStore;
        let _ = store.delete(&profile);
        let encoded = encode_bundle(&bundle("example", "TTEST", "xoxc-test-token")).unwrap();
        store.set(&profile, &encoded).unwrap();
        let loaded = store.get(&profile).unwrap().unwrap();
        let decoded = decode_bundle(&profile, &loaded).unwrap();
        assert_eq!(decoded.team_id, "TTEST");
        assert!(store.delete(&profile).unwrap());
        assert!(store.get(&profile).unwrap().is_none());
    }
}
