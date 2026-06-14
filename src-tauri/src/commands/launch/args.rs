use std::collections::HashSet;
use yaminabe_launcher_shared::datamodels::{ArgRule, ArgValue, ArgumentItem, DefaultJvmItem};

pub(super) struct LaunchVars<'a> {
    pub natives_directory: &'a str,
    pub classpath: &'a str,
    pub classpath_separator: &'a str,
    pub library_directory: &'a str,
    pub launcher_name: &'a str,
    pub launcher_version: &'a str,
    pub auth_player_name: &'a str,
    pub version_name: &'a str,
    pub game_directory: &'a str,
    pub assets_root: &'a str,
    pub assets_index_name: &'a str,
    pub auth_uuid: &'a str,
    pub auth_access_token: &'a str,
    pub user_type: &'a str,
    pub user_properties: &'a str,
    pub version_type: &'a str,
    pub clientid: &'a str,
    pub auth_xuid: &'a str,
    pub resolution_width: &'a str,
    pub resolution_height: &'a str,
}

pub(super) fn substitute_vars(s: &str, v: &LaunchVars) -> String {
    s.replace("${natives_directory}", v.natives_directory)
     .replace("${classpath_separator}", v.classpath_separator)
     .replace("${classpath}", v.classpath)
     .replace("${library_directory}", v.library_directory)
     .replace("${launcher_name}", v.launcher_name)
     .replace("${launcher_version}", v.launcher_version)
     .replace("${auth_player_name}", v.auth_player_name)
     .replace("${version_name}", v.version_name)
     .replace("${game_directory}", v.game_directory)
     .replace("${assets_root}", v.assets_root)
     .replace("${assets_index_name}", v.assets_index_name)
     .replace("${auth_uuid}", v.auth_uuid)
     .replace("${auth_access_token}", v.auth_access_token)
     .replace("${user_type}", v.user_type)
     .replace("${user_properties}", v.user_properties)
     .replace("${version_type}", v.version_type)
     .replace("${clientid}", v.clientid)
     .replace("${auth_xuid}", v.auth_xuid)
     .replace("${resolution_width}", v.resolution_width)
     .replace("${resolution_height}", v.resolution_height)
}

pub(super) fn eval_rules(rules: &[ArgRule]) -> bool {
    if rules.is_empty() { return true; }
    let mut result = false;
    for rule in rules {
        let os_ok = if let Some(vr) = &rule.os.version_range {
            vr.min.is_some() && vr.max.is_none()
        } else {
            rule.os.name.as_deref().map_or(true, |n| n == "windows")
        };
        let arch_ok = rule.os.arch.as_deref().map_or(true, |a| a != "x86");
        if os_ok && arch_ok { result = rule.action == "allow"; }
    }
    result
}

/// Stable dedup over an iterator of strings — keeps the first occurrence of
/// each value, preserving the original order of the remainder.
pub(super) fn dedup_preserve_order(items: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen: HashSet<String> = HashSet::new();
    items.into_iter().filter(|s| seen.insert(s.clone())).collect()
}

fn extend_from_arg_value(out: &mut Vec<String>, value: &ArgValue, mut map: impl FnMut(&str) -> String) {
    match value {
        ArgValue::One(s) => out.push(map(s)),
        ArgValue::Many(v) => out.extend(v.iter().map(|s| map(s))),
    }
}

pub(super) fn collect_default_jvm(items: &[DefaultJvmItem]) -> Vec<String> {
    let mut out = Vec::new();
    for item in items {
        if item.rules.iter().any(|r| r.features.is_some()) { continue; }
        if !eval_rules(&item.rules) { continue; }
        extend_from_arg_value(&mut out, &item.value, |s| s.to_string());
    }
    out
}

pub(super) fn process_args(items: &[ArgumentItem], vars: &LaunchVars) -> Vec<String> {
    let has_resolution = !vars.resolution_width.is_empty() && !vars.resolution_height.is_empty();
    let mut out = Vec::new();
    for item in items {
        match item {
            ArgumentItem::Plain(s) => out.push(substitute_vars(s, vars)),
            ArgumentItem::Conditional { rules, value } => {
                let feature_applies = rules.iter().any(|r| {
                    r.features.as_ref().map_or(false, |f| f.has_custom_resolution)
                });
                if feature_applies {
                    if has_resolution {
                        extend_from_arg_value(&mut out, value, |s| substitute_vars(s, vars));
                    }
                    continue;
                }
                if rules.iter().any(|r| r.features.is_some()) { continue; }
                if !eval_rules(rules) { continue; }
                extend_from_arg_value(&mut out, value, |s| substitute_vars(s, vars));
            }
        }
    }
    out
}