//! Session and power actions: lock, suspend, log out, restart, power off.

use crate::utils;
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemAction {
    Lock,
    Suspend,
    LogOut,
    Restart,
    PowerOff,
}

impl SystemAction {
    pub const ALL: [SystemAction; 5] = [
        SystemAction::Lock,
        SystemAction::Suspend,
        SystemAction::LogOut,
        SystemAction::Restart,
        SystemAction::PowerOff,
    ];

    pub fn name(self) -> &'static str {
        match self {
            SystemAction::Lock => "Lock",
            SystemAction::Suspend => "Suspend",
            SystemAction::LogOut => "Log Out",
            SystemAction::Restart => "Restart",
            SystemAction::PowerOff => "Power Off",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            SystemAction::Lock => "Lock the screen",
            SystemAction::Suspend => "Put the computer to sleep",
            SystemAction::LogOut => "End this session",
            SystemAction::Restart => "Restart the computer",
            SystemAction::PowerOff => "Shut the computer down",
        }
    }

    /// Lowercase words this action is found by.
    pub fn keywords(self) -> &'static [&'static str] {
        match self {
            SystemAction::Lock => &["lock", "lock screen"],
            SystemAction::Suspend => &["suspend", "sleep"],
            SystemAction::LogOut => &["log out", "logout", "sign out"],
            SystemAction::Restart => &["restart", "reboot"],
            SystemAction::PowerOff => &["power off", "poweroff", "shutdown", "shut down"],
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            SystemAction::Lock => "system-lock-screen-symbolic",
            SystemAction::Suspend => "weather-clear-night-symbolic",
            SystemAction::LogOut => "system-log-out-symbolic",
            SystemAction::Restart => "system-reboot-symbolic",
            SystemAction::PowerOff => "system-shutdown-symbolic",
        }
    }

    fn ends_session(self) -> bool {
        matches!(
            self,
            SystemAction::LogOut | SystemAction::Restart | SystemAction::PowerOff
        )
    }

    /// GNOME shows its own confirmation dialog (with a countdown) for the
    /// actions that end the session. Elsewhere there is no such dialog, so
    /// the launcher asks for a second Enter itself.
    pub fn needs_confirmation(self) -> bool {
        self.ends_session() && !utils::has_command("gnome-session-quit")
    }

    fn command(self) -> (&'static str, Vec<&'static str>) {
        let gnome = utils::has_command("gnome-session-quit");
        match self {
            SystemAction::Lock => ("loginctl", vec!["lock-session"]),
            SystemAction::Suspend => ("systemctl", vec!["suspend"]),
            SystemAction::LogOut if gnome => ("gnome-session-quit", vec!["--logout"]),
            SystemAction::Restart if gnome => ("gnome-session-quit", vec!["--reboot"]),
            SystemAction::PowerOff if gnome => ("gnome-session-quit", vec!["--power-off"]),
            SystemAction::LogOut => ("loginctl", vec!["terminate-user", ""]),
            SystemAction::Restart => ("systemctl", vec!["reboot"]),
            SystemAction::PowerOff => ("systemctl", vec!["poweroff"]),
        }
    }

    pub fn run(self) -> Result<(), String> {
        let (program, args) = self.command();
        let mut args: Vec<String> = args.into_iter().map(String::from).collect();
        if args.first().map(String::as_str) == Some("terminate-user") {
            let user = std::env::var("USER").unwrap_or_default();
            if user.is_empty() {
                return Err("Could not determine the current user".to_string());
            }
            args[1] = user;
        }
        utils::spawn_detached(Command::new(program).args(&args))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_action_is_described_and_findable() {
        for action in SystemAction::ALL {
            assert!(!action.name().is_empty());
            assert!(!action.description().is_empty());
            assert!(!action.keywords().is_empty());
            assert!(action.icon().ends_with("-symbolic"));
            assert!(action.keywords().iter().all(|k| *k == k.to_lowercase()));
        }
    }

    #[test]
    fn only_session_ending_actions_can_need_confirmation() {
        assert!(!SystemAction::Lock.needs_confirmation());
        assert!(!SystemAction::Suspend.needs_confirmation());
        assert!(SystemAction::PowerOff.ends_session());
        assert!(SystemAction::Restart.ends_session());
        assert!(SystemAction::LogOut.ends_session());
    }
}
