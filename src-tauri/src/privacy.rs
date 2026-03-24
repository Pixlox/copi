use regex::Regex;

pub fn should_capture(content: &str, app: &tauri::AppHandle) -> bool {
    let config = match crate::settings::get_config_sync(app.clone()) {
        Ok(c) => c,
        Err(_) => return true,
    };

    // Check excluded apps
    #[cfg(target_os = "macos")]
    {
        let source_app = crate::macos::get_frontmost_app_name();
        for excluded in &config.privacy.excluded_apps {
            if source_app.contains(excluded) {
                return false;
            }
        }
    }

    // Check privacy regex rules
    for pattern in &config.privacy.privacy_rules {
        if let Ok(re) = Regex::new(pattern) {
            if re.is_match(content) {
                return false;
            }
        }
    }

    true
}
