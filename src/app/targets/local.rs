// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Local devices, package export, the local install plan and failure recovery (task T18.06).
//!
//! The acceptance line is *"each target has the local tools it actually has; SSH and network
//! deployment are not in this release, and a device function that does not exist is disabled"*.
//! The type keeps the three refusals apart:
//!
//! * **[`ToolState::Missing`]** — this machine does not have the program, and there is a remedy.
//! * **[`ToolState::NotApplicable`]** — the target has no such function at all (a desktop build is
//!   not installed onto a device), so no remedy would help. This is the "disabled" the acceptance
//!   asks for, stated as a reason rather than an absent button.
//! * **[`Transport::Remote`]** — the network path is refused with the future family that owns it
//!   (F28); this build does not upload to anything.
//!
//! Also structural:
//!
//! * **A plan is not an installation.** [`Plan`] carries the command that would run and an
//!   observation is only a [`Journal`]; the module never invents a success for a program it did not
//!   start.
//! * **A failed install keeps what it replaced.** [`Recovery`] holds the previous version, and
//!   [`Recovery::rollback`] reports that there is nothing to restore when there was no previous
//!   version, rather than answering with a success shape.
//! * **Export is real I/O that reads itself back.** [`export`] writes the package file and rereads
//!   it, so the path it returns is a file that exists with the bytes it claims.

#![allow(dead_code)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use super::configuration::Os;

/// What a local tool is for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ToolKind {
    /// Listing attached devices.
    Devices,
    /// Installing onto a device.
    Install,
    /// Producing a package.
    Package,
    /// Signing a package.
    Sign,
    /// Reading a device's log.
    Logs,
}

impl ToolKind {
    /// The kind's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Devices => "devices",
            Self::Install => "install",
            Self::Package => "package",
            Self::Sign => "sign",
            Self::Logs => "logs",
        }
    }

    /// Reads a tool kind.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "devices" => Some(Self::Devices),
            "install" => Some(Self::Install),
            "package" => Some(Self::Package),
            "sign" => Some(Self::Sign),
            "logs" => Some(Self::Logs),
            _ => None,
        }
    }

    /// Every kind, in the order the report lists them.
    pub fn all() -> [Self; 5] {
        [
            Self::Devices,
            Self::Install,
            Self::Package,
            Self::Sign,
            Self::Logs,
        ]
    }
}

/// Whether a local function is available.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ToolState {
    /// The program is there.
    Present,
    /// The program is not there, with a remedy.
    Missing {
        /// What to do about it.
        remedy: String,
    },
    /// The target has no such function.
    NotApplicable {
        /// Why not.
        reason: String,
    },
}

impl ToolState {
    /// Whether the function can be used.
    pub fn usable(&self) -> bool {
        matches!(self, Self::Present)
    }
}

/// One local tool for one target.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Tool {
    /// What it is for.
    pub kind: ToolKind,
    /// The program it runs, when it has one.
    pub program: Option<String>,
    /// Whether it is there.
    pub state: ToolState,
}

impl Tool {
    /// A tool with its state.
    pub fn new(kind: ToolKind, program: impl Into<String>, state: ToolState) -> Self {
        Self {
            kind,
            program: Some(program.into()),
            state,
        }
    }

    /// A function the target does not have.
    pub fn not_applicable(kind: ToolKind, reason: impl Into<String>) -> Self {
        Self {
            kind,
            program: None,
            state: ToolState::NotApplicable {
                reason: reason.into(),
            },
        }
    }

    /// The tool in one line.
    pub fn render(&self) -> String {
        match (&self.state, &self.program) {
            (ToolState::Present, Some(program)) => format!("{}: {program}", self.kind.name()),
            (ToolState::Missing { remedy }, Some(program)) => {
                format!("{}: missing {program} (remedy: {remedy})", self.kind.name())
            }
            (ToolState::NotApplicable { reason }, _) => {
                format!("{}: not applicable ({reason})", self.kind.name())
            }
            (ToolState::Present | ToolState::Missing { .. }, None) => {
                format!("{}: no program", self.kind.name())
            }
        }
    }
}

/// The programs this machine has, as a probe reports them.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Probe {
    programs: BTreeSet<String>,
}

impl Probe {
    /// A probe over a known set of program names.
    pub fn of(programs: impl IntoIterator<Item = String>) -> Self {
        Self {
            programs: programs.into_iter().collect(),
        }
    }

    /// A probe that reads this process's `PATH`.
    pub fn from_path() -> Self {
        let mut programs = BTreeSet::new();
        if let Some(path) = std::env::var_os("PATH") {
            for directory in std::env::split_paths(&path) {
                if let Ok(entries) = std::fs::read_dir(directory) {
                    for entry in entries.flatten() {
                        programs.insert(entry.file_name().to_string_lossy().into_owned());
                    }
                }
            }
        }
        Self { programs }
    }

    /// Whether a program is there.
    pub fn has(&self, program: &str) -> bool {
        self.programs.contains(program)
    }

    fn state(&self, program: &str, remedy: &str) -> ToolState {
        if self.has(program) {
            ToolState::Present
        } else {
            ToolState::Missing {
                remedy: remedy.to_owned(),
            }
        }
    }
}

/// The mobile platform a package is for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MobilePlatform {
    /// iOS.
    Ios,
    /// Android.
    Android,
}

impl MobilePlatform {
    /// The platform's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Ios => "ios",
            Self::Android => "android",
        }
    }

    /// Reads a platform name.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "ios" => Some(Self::Ios),
            "android" => Some(Self::Android),
            _ => None,
        }
    }
}

/// The local functions a target has on this machine.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Toolbox {
    tools: Vec<Tool>,
}

impl Toolbox {
    /// Probes the local tools a profile has on a host.
    pub fn for_target(profile: &str, host: Os, platform: MobilePlatform, probe: &Probe) -> Self {
        let tools = match profile {
            "desktop" => vec![
                Tool::not_applicable(
                    ToolKind::Devices,
                    "a desktop build runs where it was built; there is no device to list",
                ),
                Tool::not_applicable(
                    ToolKind::Install,
                    "nothing is installed for a desktop build",
                ),
                Tool::new(
                    ToolKind::Package,
                    "cargo",
                    probe.state("cargo", "install Rust and put cargo on the path"),
                ),
                {
                    let (program, remedy) = match host {
                        Os::Macos => ("codesign", "install the Xcode command line tools"),
                        Os::Linux => ("gpg", "install gnupg"),
                        Os::Windows => ("signtool", "install the Windows SDK"),
                    };
                    Tool::new(ToolKind::Sign, program, probe.state(program, remedy))
                },
                Tool::not_applicable(
                    ToolKind::Logs,
                    "the desktop program's log is its own terminal",
                ),
            ],
            "tablet" | "mobile" => {
                let (device, package, sign) = match platform {
                    MobilePlatform::Ios => ("xcrun", "xcrun", "codesign"),
                    MobilePlatform::Android => ("adb", "gradle", "apksigner"),
                };
                let sdk = match platform {
                    MobilePlatform::Ios => "install Xcode and its command line tools",
                    MobilePlatform::Android => {
                        "install the Android SDK and put its tools on the path"
                    }
                };
                vec![
                    Tool::new(ToolKind::Devices, device, probe.state(device, sdk)),
                    Tool::new(ToolKind::Install, device, probe.state(device, sdk)),
                    Tool::new(ToolKind::Package, package, probe.state(package, sdk)),
                    Tool::new(ToolKind::Sign, sign, probe.state(sign, sdk)),
                    Tool::new(ToolKind::Logs, device, probe.state(device, sdk)),
                ]
            }
            // The restricted profiles are flashed, not installed; the only local seam this build
            // has is the probe tool, and signing is not part of it.
            "mini" | "embedded" => vec![
                Tool::new(
                    ToolKind::Devices,
                    "probe-rs",
                    probe.state("probe-rs", "install probe-rs to list attached probes"),
                ),
                Tool::new(
                    ToolKind::Install,
                    "probe-rs",
                    probe.state("probe-rs", "install probe-rs to flash the target"),
                ),
                Tool::new(
                    ToolKind::Package,
                    "cargo",
                    probe.state("cargo", "install Rust and put cargo on the path"),
                ),
                Tool::not_applicable(
                    ToolKind::Sign,
                    "this build produces unsigned firmware; signing is not part of the local seam",
                ),
                Tool::new(
                    ToolKind::Logs,
                    "probe-rs",
                    probe.state("probe-rs", "install probe-rs to read the target's log"),
                ),
            ],
            _ => Vec::new(),
        };
        Self { tools }
    }

    /// Every tool, in report order.
    pub fn tools(&self) -> &[Tool] {
        &self.tools
    }

    /// One tool by kind.
    pub fn get(&self, kind: ToolKind) -> Option<&Tool> {
        self.tools.iter().find(|tool| tool.kind == kind)
    }

    /// A tool this operation needs, or the refusal that says why it cannot be used.
    pub fn require(&self, kind: ToolKind) -> Result<&Tool, Refusal> {
        let Some(tool) = self.get(kind) else {
            return Err(Refusal::NotApplicable {
                kind,
                reason: "this target has no such local function".to_owned(),
            });
        };
        match &tool.state {
            ToolState::Present => Ok(tool),
            ToolState::Missing { remedy } => Err(Refusal::ToolMissing {
                kind,
                remedy: remedy.clone(),
            }),
            ToolState::NotApplicable { reason } => Err(Refusal::NotApplicable {
                kind,
                reason: reason.clone(),
            }),
        }
    }

    /// The toolbox in one line per tool.
    pub fn render(&self) -> Vec<String> {
        self.tools.iter().map(Tool::render).collect()
    }
}

/// An attached device.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Device {
    /// The id the local tool reports.
    pub id: String,
    /// The model a person recognises.
    pub model: String,
    /// Whether the local tool may act on it.
    pub authorized: bool,
}

impl Device {
    /// A device with its authorization state.
    pub fn new(id: impl Into<String>, model: impl Into<String>, authorized: bool) -> Self {
        Self {
            id: id.into(),
            model: model.into(),
            authorized,
        }
    }

    /// The device in one line.
    pub fn render(&self) -> String {
        format!(
            "{} {} ({})",
            self.id,
            self.model,
            if self.authorized {
                "authorized"
            } else {
                "not authorized"
            }
        )
    }
}

/// The devices local discovery found.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Discovery {
    devices: Vec<Device>,
}

impl Discovery {
    /// No devices yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a device, refusing a second one with the same id.
    pub fn add(&mut self, device: Device) -> Result<(), Refusal> {
        if self.devices.iter().any(|known| known.id == device.id) {
            return Err(Refusal::DuplicateDevice { id: device.id });
        }
        self.devices.push(device);
        Ok(())
    }

    /// Every device.
    pub fn devices(&self) -> &[Device] {
        &self.devices
    }

    /// Selects a device that the local tool may act on.
    pub fn select(&self, id: &str) -> Result<&Device, Refusal> {
        let Some(device) = self.devices.iter().find(|device| device.id == id) else {
            return Err(Refusal::NoSuchDevice { id: id.to_owned() });
        };
        if !device.authorized {
            return Err(Refusal::NotAuthorized {
                device: id.to_owned(),
            });
        }
        Ok(device)
    }

    /// Marks a device authorized, as a person or the platform's prompt would.
    pub fn authorize(&mut self, id: &str) -> Result<(), Refusal> {
        let Some(device) = self.devices.iter_mut().find(|device| device.id == id) else {
            return Err(Refusal::NoSuchDevice { id: id.to_owned() });
        };
        device.authorized = true;
        Ok(())
    }
}

/// How a package reaches a device.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Transport {
    /// The local tool seam this release has.
    Local,
    /// A network path, which this release does not have.
    Remote {
        /// The host it would reach.
        host: String,
    },
}

/// One package to export or install.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Package {
    /// The bundle identifier.
    pub identifier: String,
    /// The version.
    pub version: String,
    /// The profile it was built for.
    pub profile: String,
    /// The artifact on disk.
    pub path: PathBuf,
}

impl Package {
    /// A package of an identifier, a version, a profile and a path.
    pub fn new(
        identifier: impl Into<String>,
        version: impl Into<String>,
        profile: impl Into<String>,
        path: impl Into<PathBuf>,
    ) -> Self {
        Self {
            identifier: identifier.into(),
            version: version.into(),
            profile: profile.into(),
            path: path.into(),
        }
    }
}

/// A local install that would run, with the command it would use.
///
/// A plan is deliberately not an installation: it names the program and the arguments so a person
/// can read it before anything runs, and the CLI reports it as a plan rather than as a result.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Plan {
    /// The device.
    pub device: String,
    /// The package.
    pub package: Package,
    /// The program.
    pub program: String,
    /// The command line, as one string.
    pub command: String,
}

impl Plan {
    /// The plan in one line.
    pub fn render(&self) -> String {
        format!(
            "install {} {} on {}: {}",
            self.package.identifier, self.package.version, self.device, self.command
        )
    }
}

/// Plans a local installation, refusing every reason it cannot happen.
pub fn plan_install(
    toolbox: &Toolbox,
    discovery: &Discovery,
    device: &str,
    package: &Package,
    transport: &Transport,
) -> Result<Plan, Refusal> {
    if let Transport::Remote { host } = transport {
        return Err(Refusal::RemoteUnsupported { host: host.clone() });
    }
    let tool = toolbox.require(ToolKind::Install)?;
    discovery.select(device)?;
    if !package.path.exists() {
        return Err(Refusal::NoArtifact {
            path: package.path.display().to_string(),
        });
    }
    let program = tool.program.clone().ok_or_else(|| Refusal::NotApplicable {
        kind: ToolKind::Install,
        reason: "the install tool has no program".to_owned(),
    })?;
    let artifact = package.path.display().to_string();
    let command = match program.as_str() {
        "adb" => format!("adb install -r {artifact}"),
        "xcrun" => format!("xcrun simctl install {device} {artifact}"),
        "probe-rs" => format!("probe-rs download {artifact}"),
        other => format!("{other} {artifact}"),
    };
    Ok(Plan {
        device: device.to_owned(),
        package: package.clone(),
        program,
        command,
    })
}

/// Writes a package file and reads it back.
///
/// The directory is created if needed. The returned path is the file that was written **and
/// reread**; a write that could not be read back is a refusal, not a path.
pub fn export(package: &Package, directory: &Path) -> Result<PathBuf, Refusal> {
    let path = directory.join(format!("{}.rustage-package", package.identifier));
    let text = format!(
        "rustage-package 1\nprofile {}\nidentifier {}\nversion {}\nartifact {}\n",
        package.profile,
        package.identifier,
        package.version,
        package.path.display()
    );
    let failed = |reason: String| Refusal::ExportFailed {
        path: path.display().to_string(),
        reason,
    };
    std::fs::create_dir_all(directory).map_err(|error| failed(error.to_string()))?;
    std::fs::write(&path, &text).map_err(|error| failed(error.to_string()))?;
    let read_back = std::fs::read_to_string(&path).map_err(|error| failed(error.to_string()))?;
    if read_back != text {
        return Err(failed("the file read back differently".to_owned()));
    }
    Ok(path)
}

/// One step of an install attempt.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Step {
    /// What was attempted.
    pub name: String,
    /// Why it stopped, when it did.
    pub failure: Option<String>,
}

/// The steps one install attempt took.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Journal {
    steps: Vec<Step>,
}

impl Journal {
    /// An empty journal.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a step that succeeded.
    pub fn succeeded(&mut self, name: impl Into<String>) {
        self.steps.push(Step {
            name: name.into(),
            failure: None,
        });
    }

    /// Records the step that failed; later steps are not attempted.
    pub fn failed(&mut self, name: impl Into<String>, reason: impl Into<String>) {
        self.steps.push(Step {
            name: name.into(),
            failure: Some(reason.into()),
        });
    }

    /// Every step.
    pub fn steps(&self) -> &[Step] {
        &self.steps
    }

    /// Whether any step failed.
    pub fn is_failed(&self) -> bool {
        self.steps.iter().any(|step| step.failure.is_some())
    }

    /// The failure in one line, when there is one.
    pub fn failure(&self) -> Option<String> {
        self.steps.iter().find_map(|step| {
            step.failure
                .as_ref()
                .map(|reason| format!("{} failed: {reason}", step.name))
        })
    }
}

/// What an install attempt can restore after a failure.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Recovery {
    /// The device.
    pub device: String,
    /// The version that was installed before the attempt, when there was one.
    pub previous: Option<String>,
    /// What the attempt did.
    pub journal: Journal,
}

impl Recovery {
    /// What recovery can restore.
    ///
    /// `Err` when there was no previous version: "nothing to restore" is a fact about the device,
    /// not a silent success.
    pub fn rollback(&self) -> Result<String, String> {
        match &self.previous {
            Some(version) => Ok(format!(
                "restored {version} on {} after: {}",
                self.device,
                self.journal
                    .failure()
                    .unwrap_or_else(|| "the attempt stopped".to_owned())
            )),
            None => Err(format!(
                "nothing was installed on {} before this attempt",
                self.device
            )),
        }
    }
}

/// Why a local operation cannot happen.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Refusal {
    /// The program is not on this machine.
    ToolMissing {
        /// Which function.
        kind: ToolKind,
        /// What to do.
        remedy: String,
    },
    /// The target has no such function at all.
    NotApplicable {
        /// Which function.
        kind: ToolKind,
        /// Why not.
        reason: String,
    },
    /// No attached device has this id.
    NoSuchDevice {
        /// The id.
        id: String,
    },
    /// The device has not been authorized.
    NotAuthorized {
        /// The device.
        device: String,
    },
    /// The same device was discovered twice.
    DuplicateDevice {
        /// The id.
        id: String,
    },
    /// A network install was asked for.
    RemoteUnsupported {
        /// The host it would reach.
        host: String,
    },
    /// The artifact the package names is not there.
    NoArtifact {
        /// The path.
        path: String,
    },
    /// The package file could not be written or read back.
    ExportFailed {
        /// The path.
        path: String,
        /// Why.
        reason: String,
    },
}

impl Refusal {
    /// The refusal in one line.
    pub fn render(&self) -> String {
        match self {
            Self::ToolMissing { kind, remedy } => {
                format!(
                    "{}: the local tool is missing (remedy: {remedy})",
                    kind.name()
                )
            }
            Self::NotApplicable { kind, reason } => {
                format!("{}: disabled for this target ({reason})", kind.name())
            }
            Self::NoSuchDevice { id } => format!("there is no device {id}"),
            Self::NotAuthorized { device } => {
                format!("{device} is not authorized; authorize it before installing")
            }
            Self::DuplicateDevice { id } => format!("device {id} was already discovered"),
            Self::RemoteUnsupported { host } => format!(
                "remote deployment to {host} is not part of this release; it belongs to F28"
            ),
            Self::NoArtifact { path } => format!("there is no artifact at {path}"),
            Self::ExportFailed { path, reason } => {
                format!("{path} could not be written and read back: {reason}")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Device, Discovery, Journal, MobilePlatform, Package, Probe, Recovery, Refusal, ToolKind,
        ToolState, Toolbox, Transport, export, plan_install,
    };
    use crate::app::targets::configuration::Os;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    static SERIAL: AtomicU32 = AtomicU32::new(0);

    fn sandbox(name: &str) -> PathBuf {
        let id = SERIAL.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "rustage-target-local-{}-{id}-{name}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).expect("a temporary directory is creatable");
        path
    }

    fn android(programs: &[&str]) -> Toolbox {
        Toolbox::for_target(
            "mobile",
            Os::Macos,
            MobilePlatform::Android,
            &Probe::of(programs.iter().map(|program| (*program).to_owned())),
        )
    }

    #[test]
    fn tool_kinds_round_trip_through_their_names() {
        for kind in ToolKind::all() {
            assert_eq!(ToolKind::parse(kind.name()), Some(kind));
        }
        assert!(ToolKind::parse("upload").is_none());
    }

    #[test]
    fn a_desktop_target_has_no_device_function() {
        let toolbox =
            Toolbox::for_target("desktop", Os::Macos, MobilePlatform::Ios, &Probe::of([]));
        let device = toolbox.get(ToolKind::Devices).expect("a device entry");
        assert!(matches!(device.state, ToolState::NotApplicable { .. }));
        assert!(matches!(
            toolbox.require(ToolKind::Devices),
            Err(Refusal::NotApplicable { .. })
        ));
        assert!(toolbox.require(ToolKind::Package).is_err());
    }

    #[test]
    fn a_missing_tool_is_missing_with_a_remedy_and_a_present_one_is_usable() {
        let missing = android(&[]);
        let refusal = missing.require(ToolKind::Install).expect_err("no adb");
        match refusal {
            Refusal::ToolMissing { kind, remedy } => {
                assert_eq!(kind, ToolKind::Install);
                assert!(remedy.contains("Android SDK"));
            }
            other => panic!("expected a missing tool, got {other:?}"),
        }
        let present = android(&["adb"]);
        assert!(present.require(ToolKind::Install).is_ok());
        assert!(
            present.require(ToolKind::Sign).is_err(),
            "apksigner is absent"
        );
    }

    #[test]
    fn a_device_must_exist_and_be_authorized() {
        let mut discovery = Discovery::new();
        discovery
            .add(Device::new("emulator-5554", "Pixel 8", false))
            .expect("a device");
        assert!(matches!(
            discovery.select("nope"),
            Err(Refusal::NoSuchDevice { .. })
        ));
        assert!(matches!(
            discovery.select("emulator-5554"),
            Err(Refusal::NotAuthorized { .. })
        ));
        discovery
            .authorize("emulator-5554")
            .expect("an authorization");
        assert_eq!(
            discovery
                .select("emulator-5554")
                .expect("an authorized device")
                .model,
            "Pixel 8"
        );
    }

    #[test]
    fn a_device_cannot_be_discovered_twice() {
        let mut discovery = Discovery::new();
        discovery
            .add(Device::new("a", "A", true))
            .expect("a device");
        assert!(matches!(
            discovery.add(Device::new("a", "A", true)),
            Err(Refusal::DuplicateDevice { .. })
        ));
        assert_eq!(discovery.devices().len(), 1);
    }

    #[test]
    fn a_remote_install_is_refused_before_any_tool_is_asked_for() {
        let toolbox = android(&["adb"]);
        let mut discovery = Discovery::new();
        discovery
            .add(Device::new("d", "D", true))
            .expect("a device");
        let package = Package::new("com.example.app", "1.0.0", "mobile", "/tmp/none.apk");
        let refusal = plan_install(
            &toolbox,
            &discovery,
            "d",
            &package,
            &Transport::Remote {
                host: "build.example".to_owned(),
            },
        )
        .expect_err("a remote install");
        assert!(matches!(refusal, Refusal::RemoteUnsupported { .. }));
        assert!(refusal.render().contains("F28"));
    }

    #[test]
    fn a_plan_needs_the_artifact_and_names_the_local_command() {
        let toolbox = android(&["adb"]);
        let mut discovery = Discovery::new();
        discovery
            .add(Device::new("d", "D", true))
            .expect("a device");
        let directory = sandbox("plan");
        let artifact = directory.join("app.apk");
        std::fs::write(&artifact, b"apk").expect("a temporary artifact");
        let package = Package::new("com.example.app", "1.0.0", "mobile", &artifact);
        let missing = Package::new(
            "com.example.app",
            "1.0.0",
            "mobile",
            directory.join("missing.apk"),
        );
        assert!(matches!(
            plan_install(&toolbox, &discovery, "d", &missing, &Transport::Local),
            Err(Refusal::NoArtifact { .. })
        ));
        let plan =
            plan_install(&toolbox, &discovery, "d", &package, &Transport::Local).expect("a plan");
        assert_eq!(plan.program, "adb");
        assert!(plan.command.starts_with("adb install -r "));
        assert!(plan.render().contains("com.example.app"));
    }

    #[test]
    fn a_package_is_exported_and_reads_back() {
        let directory = sandbox("export");
        let package = Package::new(
            "com.example.app",
            "1.0.0",
            "mobile",
            directory.join("app.apk"),
        );
        let path = export(&package, &directory).expect("an export");
        assert!(path.exists());
        let text = std::fs::read_to_string(&path).expect("the package file");
        assert!(text.starts_with("rustage-package 1\n"));
        assert!(text.contains("version 1.0.0"));
    }

    #[test]
    fn a_failed_install_keeps_what_it_replaced() {
        let mut journal = Journal::new();
        journal.succeeded("copy package");
        journal.failed("install", "the device rejected the package");
        assert!(journal.is_failed());
        let recovery = Recovery {
            device: "pixel-8".to_owned(),
            previous: Some("0.9.0".to_owned()),
            journal,
        };
        let message = recovery.rollback().expect("a previous version");
        assert!(message.contains("restored 0.9.0"));
        assert!(message.contains("rejected the package"));
    }

    #[test]
    fn a_failed_first_install_has_nothing_to_restore() {
        let mut journal = Journal::new();
        journal.failed("install", "the device rejected the package");
        let recovery = Recovery {
            device: "pixel-8".to_owned(),
            previous: None,
            journal,
        };
        assert!(recovery.rollback().is_err());
    }

    #[test]
    fn the_toolbox_only_offers_the_platforms_own_programs() {
        let ios = Toolbox::for_target("mobile", Os::Macos, MobilePlatform::Ios, &Probe::of([]));
        assert_eq!(
            ios.get(ToolKind::Devices)
                .and_then(|tool| tool.program.clone()),
            Some("xcrun".to_owned())
        );
        let embedded = Toolbox::for_target(
            "embedded",
            Os::Linux,
            MobilePlatform::Android,
            &Probe::of([]),
        );
        assert!(matches!(
            embedded.get(ToolKind::Sign).map(|tool| &tool.state),
            Some(ToolState::NotApplicable { .. })
        ));
        assert_eq!(
            embedded
                .get(ToolKind::Install)
                .and_then(|tool| tool.program.clone()),
            Some("probe-rs".to_owned())
        );
    }
}
