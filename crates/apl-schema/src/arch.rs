///
/// APL supports both Apple Silicon (ARM64) and Intel (`x86_64`) Macs.
/// The architecture is used to select the correct pre-compiled binary
/// from the package index.
///
/// # Example
///
/// ```
/// use apl_schema::Arch;
///
/// let current = Arch::current();
/// println!("Running on: {}", current);
/// ```
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize, Default,
)]
#[serde(rename_all = "lowercase")]
pub enum Arch {
    /// ARM64 architecture (Apple Silicon: M1, M2, M3, etc.)
    #[default]
    Arm64,
    /// `x86_64` architecture (Intel Macs)
    X86_64,
    /// Universal binary (works on both architectures)
    Universal,
}

impl Arch {
    /// Get the current architecture
    pub fn current() -> Self {
        #[cfg(target_arch = "aarch64")]
        {
            Self::Arm64
        }
        #[cfg(not(target_arch = "aarch64"))]
        {
            Self::X86_64
        }
    }

    /// Convert to string representation
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Arm64 => "arm64",
            Self::X86_64 => "x86_64",
            Self::Universal => "universal",
        }
    }

    /// Rust-convention architecture name (`aarch64` / `x86_64`).
    ///
    /// Distinct from [`as_str()`](Self::as_str) which uses platform names
    /// (`arm64`). The value matches `std::env::consts::ARCH` and is
    /// exposed to build scripts as the `$ARCH` environment variable.
    pub fn rust_name(&self) -> &'static str {
        match self {
            Self::Arm64 => "aarch64",
            Self::X86_64 => "x86_64",
            Self::Universal => "universal",
        }
    }
}

impl std::fmt::Display for Arch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for Arch {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "arm64" | "aarch64" | "arm64-macos" => Ok(Self::Arm64),
            "x86_64" | "amd64" | "x86_64-macos" => Ok(Self::X86_64),
            "universal" | "universal-macos" => Ok(Self::Universal),
            _ => Err(format!("Unknown architecture: {s}")),
        }
    }
}
