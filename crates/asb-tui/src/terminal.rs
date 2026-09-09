// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Deterministic terminal capability evidence and conservative render policy.

use serde::{Deserialize, Serialize};
use std::env;
use std::fmt;
use std::io::{IsTerminal, stdin, stdout};

/// Maximum terminal dimension accepted from an environment hint.
pub const MAX_TERMINAL_DIMENSION: u16 = 16_384;

/// Evidence supplied by the channel and terminal, kept separate from policy.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalEvidence {
    /// Whether stdin and stdout are attached to a TTY.
    pub tty: bool,
    /// TERM value, if present and bounded.
    pub term: Option<String>,
    /// Terminal program value, if present and bounded.
    pub term_program: Option<String>,
    /// Color capability hint, if present and bounded.
    pub color_term: Option<String>,
    /// Whether the channel advertises an SSH session.
    pub ssh: bool,
    /// Whether a tmux multiplexer is present.
    pub tmux: bool,
    /// Whether a screen multiplexer is present.
    pub screen: bool,
    /// Whether color output was explicitly disabled.
    pub no_color: bool,
    /// Width hint, when supplied by the channel.
    pub columns: Option<u16>,
    /// Height hint, when supplied by the channel.
    pub lines: Option<u16>,
}

impl TerminalEvidence {
    /// Capture only bounded, non-secret process environment hints and TTY state.
    pub fn from_environment() -> Self {
        Self {
            tty: stdin().is_terminal() && stdout().is_terminal(),
            term: bounded_env("TERM"),
            term_program: bounded_env("TERM_PROGRAM"),
            color_term: bounded_env("COLORTERM"),
            ssh: env::var_os("SSH_CONNECTION").is_some() || env::var_os("SSH_TTY").is_some(),
            tmux: env::var_os("TMUX").is_some(),
            screen: env::var_os("STY").is_some(),
            no_color: env::var_os("NO_COLOR").is_some(),
            columns: bounded_dimension("COLUMNS"),
            lines: bounded_dimension("LINES"),
        }
    }

    /// Validate public evidence before it is shown by the doctor command.
    pub fn validate(&self) -> Result<(), TerminalError> {
        for value in [&self.term, &self.term_program, &self.color_term]
            .into_iter()
            .flatten()
        {
            if value.is_empty() || value.len() > 128 || value.chars().any(char::is_control) {
                return Err(TerminalError::InvalidEvidence);
            }
        }
        for dimension in [self.columns, self.lines].into_iter().flatten() {
            if dimension == 0 || dimension > MAX_TERMINAL_DIMENSION {
                return Err(TerminalError::InvalidEvidence);
            }
        }
        Ok(())
    }
}

fn bounded_env(name: &str) -> Option<String> {
    env::var(name).ok().filter(|value| {
        !value.is_empty() && value.len() <= 128 && !value.chars().any(char::is_control)
    })
}

fn bounded_dimension(name: &str) -> Option<u16> {
    env::var(name)
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .filter(|value| *value > 0 && *value <= MAX_TERMINAL_DIMENSION)
}

/// Conservative rendering capability tier.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityTier {
    /// Plain ASCII and no color, suitable for pipes and unknown channels.
    Plain,
    /// ANSI 8/16-color output with conservative Unicode.
    BasicColor,
    /// 256-color output with Unicode when width behavior is known.
    IndexedColor,
    /// Truecolor and enhanced input after explicit evidence.
    TrueColor,
}

/// Feature gates selected from terminal evidence.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenderPolicy {
    /// Selected color/Unicode capability tier.
    pub tier: CapabilityTier,
    /// Whether Unicode glyphs may be emitted.
    pub unicode: bool,
    /// Whether optional mouse input may be enabled.
    pub mouse: bool,
    /// Whether focus reporting may be enabled.
    pub focus: bool,
    /// Whether bracketed paste may be enabled.
    pub bracketed_paste: bool,
    /// Whether synchronized output is safe to request.
    pub synchronized_output: bool,
    /// Whether alternate-screen mode is allowed.
    pub alternate_screen: bool,
}

impl RenderPolicy {
    /// Select a fail-closed policy from validated evidence.
    pub fn from_evidence(evidence: &TerminalEvidence) -> Result<Self, TerminalError> {
        evidence.validate()?;
        let term = evidence.term.as_deref().unwrap_or("");
        let color = evidence.color_term.as_deref().unwrap_or("");
        let indexed = term.contains("256color") || color.eq_ignore_ascii_case("256");
        let truecolor = color.eq_ignore_ascii_case("truecolor")
            || color.eq_ignore_ascii_case("24bit")
            || term.contains("direct");
        let tier = if !evidence.tty || evidence.no_color {
            CapabilityTier::Plain
        } else if truecolor {
            CapabilityTier::TrueColor
        } else if indexed {
            CapabilityTier::IndexedColor
        } else {
            CapabilityTier::BasicColor
        };
        let enhanced = matches!(
            tier,
            CapabilityTier::IndexedColor | CapabilityTier::TrueColor
        ) && evidence.tty
            && !evidence.tmux
            && !evidence.screen;
        Ok(Self {
            tier,
            unicode: !matches!(tier, CapabilityTier::Plain),
            mouse: enhanced,
            focus: enhanced,
            bracketed_paste: enhanced,
            synchronized_output: matches!(tier, CapabilityTier::TrueColor) && !evidence.ssh,
            alternate_screen: evidence.tty,
        })
    }

    /// Compact, stable diagnostic suitable for `doctor` output.
    pub fn doctor_line(&self, evidence: &TerminalEvidence) -> String {
        format!(
            "tier={:?} tty={} channel={} size={}x{} unicode={} mouse={} focus={} no_color={}",
            self.tier,
            evidence.tty,
            channel(evidence),
            evidence
                .columns
                .map_or_else(|| "?".into(), |v| v.to_string()),
            evidence.lines.map_or_else(|| "?".into(), |v| v.to_string()),
            self.unicode,
            self.mouse,
            self.focus,
            evidence.no_color
        )
    }
}

fn channel(evidence: &TerminalEvidence) -> &'static str {
    if evidence.tmux {
        "tmux"
    } else if evidence.screen {
        "screen"
    } else if evidence.ssh {
        "ssh"
    } else if evidence.tty {
        "local-tty"
    } else {
        "non-tty"
    }
}

/// Terminal evidence or policy was malformed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalError {
    /// An untrusted hint exceeded the public bound or contained control data.
    InvalidEvidence,
}

impl fmt::Display for TerminalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("invalid terminal capability evidence")
    }
}

impl std::error::Error for TerminalError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn evidence() -> TerminalEvidence {
        TerminalEvidence {
            tty: true,
            term: Some("xterm-256color".into()),
            term_program: Some("WezTerm".into()),
            color_term: Some("truecolor".into()),
            ssh: false,
            tmux: false,
            screen: false,
            no_color: false,
            columns: Some(120),
            lines: Some(40),
        }
    }

    #[test]
    fn policy_requires_evidence_and_enables_only_supported_features() {
        let policy = RenderPolicy::from_evidence(&evidence()).unwrap();
        assert_eq!(policy.tier, CapabilityTier::TrueColor);
        assert!(policy.unicode && policy.mouse && policy.focus);
        assert!(policy.synchronized_output);
        assert!(
            policy
                .doctor_line(&evidence())
                .contains("channel=local-tty")
        );
    }

    #[test]
    fn multiplexers_and_ssh_disable_risky_enhancements() {
        let mut value = evidence();
        value.tmux = true;
        value.ssh = true;
        let policy = RenderPolicy::from_evidence(&value).unwrap();
        assert!(!policy.mouse && !policy.focus && !policy.synchronized_output);
    }

    #[test]
    fn pipes_and_no_color_are_plain_and_bounded() {
        let mut value = evidence();
        value.tty = false;
        value.no_color = true;
        value.columns = Some(0);
        assert_eq!(
            RenderPolicy::from_evidence(&value),
            Err(TerminalError::InvalidEvidence)
        );
        value.columns = None;
        let policy = RenderPolicy::from_evidence(&value).unwrap();
        assert_eq!(policy.tier, CapabilityTier::Plain);
        assert!(!policy.unicode && !policy.alternate_screen);
    }

    #[test]
    fn control_values_and_oversized_dimensions_fail_closed() {
        let mut value = evidence();
        value.term = Some("xterm\n".into());
        assert_eq!(value.validate(), Err(TerminalError::InvalidEvidence));
        value.term = Some("xterm".into());
        value.lines = Some(MAX_TERMINAL_DIMENSION + 1);
        assert_eq!(value.validate(), Err(TerminalError::InvalidEvidence));
    }
}
