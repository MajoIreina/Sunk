//! Serializable settings with safe defaults and validation.

use serde::{Deserialize, Serialize};
use std::error::Error;
use std::fmt;

pub const CURRENT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub schema_version: u32,
    pub rendering: RenderingSettings,
    pub file_operations: FileOperationSettings,
    pub window: WindowSettings,
    pub launch_on_startup: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            rendering: RenderingSettings::default(),
            file_operations: FileOperationSettings::default(),
            window: WindowSettings::default(),
            launch_on_startup: false,
        }
    }
}

impl Settings {
    /// Parses and validates settings from TOML.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid TOML, unknown fields, or values outside the supported range.
    pub fn from_toml_str(source: &str) -> Result<Self, SettingsError> {
        let settings: Self = toml::from_str(source).map_err(SettingsError::Decode)?;
        settings.validate().map_err(SettingsError::Validation)?;
        Ok(settings)
    }

    /// Serializes validated settings as readable TOML.
    ///
    /// # Errors
    ///
    /// Returns an error if the settings are invalid or serialization fails.
    pub fn to_toml_string_pretty(&self) -> Result<String, SettingsError> {
        self.validate().map_err(SettingsError::Validation)?;
        toml::to_string_pretty(self).map_err(SettingsError::Encode)
    }

    /// Checks schema compatibility, value ranges, and destructive-operation policy.
    ///
    /// # Errors
    ///
    /// Returns the first validation rule violated by this settings value.
    pub fn validate(&self) -> Result<(), SettingsValidationError> {
        if self.schema_version != CURRENT_SCHEMA_VERSION {
            return Err(SettingsValidationError::UnsupportedSchemaVersion {
                found: self.schema_version,
                supported: CURRENT_SCHEMA_VERSION,
            });
        }
        if !(30..=240).contains(&self.rendering.active_fps) {
            return Err(SettingsValidationError::ActiveFpsOutOfRange(
                self.rendering.active_fps,
            ));
        }
        if !(1..=60).contains(&self.rendering.idle_fps) {
            return Err(SettingsValidationError::IdleFpsOutOfRange(
                self.rendering.idle_fps,
            ));
        }
        if self.rendering.idle_fps > self.rendering.active_fps {
            return Err(SettingsValidationError::IdleFpsExceedsActive {
                idle: self.rendering.idle_fps,
                active: self.rendering.active_fps,
            });
        }
        if !(0.25..=4.0).contains(&self.window.scale) {
            return Err(SettingsValidationError::WindowScaleOutOfRange(
                self.window.scale,
            ));
        }
        if self.file_operations.default_action == FileAction::PermanentDelete
            && !self.file_operations.permanent_delete_enabled
        {
            return Err(SettingsValidationError::PermanentDeleteRequiresOptIn);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RenderingSettings {
    pub quality: QualityPreference,
    pub active_fps: u32,
    pub idle_fps: u32,
}

impl Default for RenderingSettings {
    fn default() -> Self {
        Self {
            quality: QualityPreference::Automatic,
            active_fps: 60,
            idle_fps: 15,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QualityPreference {
    #[default]
    Automatic,
    Cinematic,
    High,
    Balanced,
    Performance,
    Background,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FileOperationSettings {
    pub default_action: FileAction,
    pub permanent_delete_enabled: bool,
}

impl Default for FileOperationSettings {
    fn default() -> Self {
        Self {
            default_action: FileAction::MoveToTrash,
            permanent_delete_enabled: false,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileAction {
    #[default]
    MoveToTrash,
    PermanentDelete,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WindowSettings {
    pub always_on_top: bool,
    pub click_through_when_idle: bool,
    pub scale: f32,
}

impl Default for WindowSettings {
    fn default() -> Self {
        Self {
            always_on_top: true,
            click_through_when_idle: false,
            scale: 1.0,
        }
    }
}

#[derive(Debug)]
pub enum SettingsError {
    Decode(toml::de::Error),
    Encode(toml::ser::Error),
    Validation(SettingsValidationError),
}

impl fmt::Display for SettingsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Decode(error) => write!(formatter, "could not parse settings: {error}"),
            Self::Encode(error) => write!(formatter, "could not encode settings: {error}"),
            Self::Validation(error) => error.fmt(formatter),
        }
    }
}

impl Error for SettingsError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Decode(error) => Some(error),
            Self::Encode(error) => Some(error),
            Self::Validation(error) => Some(error),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum SettingsValidationError {
    UnsupportedSchemaVersion { found: u32, supported: u32 },
    ActiveFpsOutOfRange(u32),
    IdleFpsOutOfRange(u32),
    IdleFpsExceedsActive { idle: u32, active: u32 },
    WindowScaleOutOfRange(f32),
    PermanentDeleteRequiresOptIn,
}

impl fmt::Display for SettingsValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedSchemaVersion { found, supported } => write!(
                formatter,
                "unsupported settings schema {found}; this build supports {supported}"
            ),
            Self::ActiveFpsOutOfRange(value) => {
                write!(
                    formatter,
                    "rendering.active_fps must be in 30..=240, got {value}"
                )
            }
            Self::IdleFpsOutOfRange(value) => {
                write!(
                    formatter,
                    "rendering.idle_fps must be in 1..=60, got {value}"
                )
            }
            Self::IdleFpsExceedsActive { idle, active } => write!(
                formatter,
                "rendering.idle_fps ({idle}) cannot exceed active_fps ({active})"
            ),
            Self::WindowScaleOutOfRange(value) => {
                write!(formatter, "window.scale must be in 0.25..=4.0, got {value}")
            }
            Self::PermanentDeleteRequiresOptIn => formatter.write_str(
                "permanent_delete requires file_operations.permanent_delete_enabled = true",
            ),
        }
    }
}

impl Error for SettingsValidationError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_safe_and_valid() {
        let settings = Settings::default();
        settings.validate().unwrap();
        assert_eq!(
            settings.file_operations.default_action,
            FileAction::MoveToTrash
        );
        assert!(!settings.file_operations.permanent_delete_enabled);
        assert!(!settings.launch_on_startup);
    }

    #[test]
    fn partial_toml_uses_defaults() {
        let settings = Settings::from_toml_str(
            r#"
                [rendering]
                quality = "high"
                active_fps = 120
            "#,
        )
        .unwrap();

        assert_eq!(settings.rendering.quality, QualityPreference::High);
        assert_eq!(settings.rendering.active_fps, 120);
        assert_eq!(settings.rendering.idle_fps, 15);
        assert_eq!(
            settings.file_operations.default_action,
            FileAction::MoveToTrash
        );
    }

    #[test]
    fn settings_round_trip_through_toml() {
        let settings = Settings::default();
        let encoded = settings.to_toml_string_pretty().unwrap();
        let decoded = Settings::from_toml_str(&encoded).unwrap();
        assert_eq!(decoded, settings);
    }

    #[test]
    fn permanent_delete_requires_explicit_opt_in() {
        let error = Settings::from_toml_str(
            r#"
                [file_operations]
                default_action = "permanent_delete"
            "#,
        )
        .unwrap_err();

        assert!(matches!(
            error,
            SettingsError::Validation(SettingsValidationError::PermanentDeleteRequiresOptIn)
        ));
    }

    #[test]
    fn invalid_frame_rate_is_rejected() {
        let mut settings = Settings::default();
        settings.rendering.active_fps = 10;
        assert_eq!(
            settings.validate(),
            Err(SettingsValidationError::ActiveFpsOutOfRange(10))
        );
    }

    #[test]
    fn unknown_fields_are_rejected() {
        let error = Settings::from_toml_str("mystery = true").unwrap_err();
        assert!(matches!(error, SettingsError::Decode(_)));
    }
}
