use std::env;
use std::fs;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{exit, Command};

#[derive(Debug)]
struct Config {
    menu: String,
    title: String,
    backtext: String,
    back: bool,
    no_result: Option<(String, String)>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            menu: String::new(),
            title: String::new(),
            backtext: "Back".to_string(),
            back: true,
            no_result: None,
        }
    }
}

#[derive(Debug)]
struct Item {
    name: String,
    action: Action,
    icon: Option<String>,
}

#[derive(Debug)]
struct Function {
    argument: String,
    command: String,
}

#[derive(Debug)]
enum Action {
    Exec(String),
    Submenu(String),
    Back,
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() != 2 {
        eprintln!("usage: menupp <file.mpp>");
        exit(1);
    }

    run_menu(Path::new(&args[1]), false);
}

fn run_menu(path: &Path, add_back: bool) {
    let content = fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!("failed to read {}: {}", path.display(), e);
        exit(1);
    });
    run_menu_content(path, &content, add_back, None);
}

fn run_menu_content(path: &Path, content: &str, add_back: bool, section: Option<&str>) {
    let (config, mut items) = match section {
        Some(section) => parse_named_section(content, section),
        None => parse(content),
    };

    if add_back && config.back {
        items.insert(
            0,
            Item {
                name: config.backtext.clone(),
                action: Action::Back,
                icon: None,
            },
        );
    }
    if items.is_empty() {
        eprintln!("no items found in {}", path.display());
        exit(1);
    }

    loop {
        let choice = run_launcher(&config, &items);

        let Some(choice) = choice else {
            return;
        };
        let Some(item) = items.iter().find(|i| i.name == choice) else {
            if let Some((_, command)) = &config.no_result {
                run_command(&command.replace("${term}", &choice));
            }
            return;
        };

        match &item.action {
            Action::Exec(command) => {
                run_command(command.as_str());
                return;
            }
            Action::Submenu(submenu) => {
                if let Some(section) = submenu.strip_prefix('*') {
                    run_menu_content(path, content, true, Some(section));
                } else {
                    let submenu_path = path
                        .parent()
                        .unwrap_or_else(|| Path::new("."))
                        .join(submenu);
                    run_menu(&submenu_path, true);
                }
            }

            Action::Back => return,
        }
    }
}

fn parse_named_section(content: &str, name: &str) -> (Config, Vec<Item>) {
    let mut section_lines = Vec::new();
    let mut in_section = false;

    for raw_line in content.lines() {
        let line = raw_line.trim();
        if let Some(section_name) = line
            .strip_prefix('*')
            .and_then(|line| line.strip_suffix(':'))
        {
            in_section = section_name == name;
            continue;
        }
        if in_section {
            section_lines.push(raw_line);
        }
    }

    parse(&section_lines.join("\n"))
}

fn parse(content: &str) -> (Config, Vec<Item>) {
    let mut config = Config::default();
    let mut items = Vec::new();
    let mut functions = HashMap::new();

    #[derive(PartialEq)]
    enum Block {
        None,
        Config,
        Functions,
        Items,
        ItemsJSON,
        NoResultFound,
    }

    let mut block = Block::None;

    for raw_line in content.lines() {
        let line = raw_line.trim();

        if line.is_empty() {
            continue;
        }

        if line.starts_with('#') {
            continue;
        }

        if line.starts_with('*') && line.ends_with(':') {
            break;
        }

        if line == "Config:" {
            block = Block::Config;
            continue;
        }

        if line == "Items:" {
            block = Block::Items;
            continue;
        }

        if line == "Functions:" {
            block = Block::Functions;
            continue;
        }

        if line == "ItemsJSON:" {
            block = Block::ItemsJSON;
            continue;
        }

        if line == "No-result-found:" {
            block = Block::NoResultFound;
            continue;
        }

        match block {
            Block::Config => {
                if let Some((key, value)) = parse_kv(line) {
                    match key.as_str() {
                        "menu" => {
                            config.menu = strip_quotes(&value).unwrap_or(value);
                        }
                        "title" => {
                            config.title = strip_quotes(&value).unwrap_or(value);
                        }
                        "backtext" => {
                            config.backtext = strip_quotes(&value).unwrap_or(value);
                        }
                        "back" => {
                            if strip_quotes(&value).as_deref() == Some("false")
                                || value.trim() == "false"
                            {
                                config.back = false;
                            }
                        }
                        _ => {}
                    }
                }
            }
            Block::Functions => {
                if let Some((name, argument, value)) = parse_function_definition(line) {
                    if let Some(command) = parse_exec(&value) {
                        functions.insert(name, Function { argument, command });
                    }
                }
            }
            Block::Items => {
                if let Some((key, value)) = parse_kv(line) {
                    let action = parse_action(&value, &functions);
                    if let Some(action) = action {
                        items.push(Item {
                            name: key,
                            action,
                            icon: None,
                        });
                    }
                }
            }
            Block::ItemsJSON => {
                if line == "applist()" || line == "applist(noIcons)" {
                    items.extend(parse_items_json(line));
                } else if let Some(command) = parse_exec(line) {
                    items.extend(parse_items_json(&command));
                }
            }
            Block::NoResultFound => {
                if let Some((label, value)) = parse_kv(line) {
                    if let Some(command) = parse_exec(&value) {
                        config.no_result = Some((label, command));
                    }
                }
            }
            Block::None => {}
        }

    }

    fn parse_action(value: &str, functions: &HashMap<String, Function>) -> Option<Action> {
        if let Some(command) = parse_exec(value) {
            return Some(Action::Exec(command));
        }
        if let Some(submenu) = parse_submenu(value) {
            return Some(Action::Submenu(submenu));
        }
        let (name, argument) = parse_function_call(value)?;
        let function = functions.get(&name)?;
        Some(Action::Exec(
            function
                .command
                .replace(&format!("${{{}}}", function.argument), &argument),
        ))
    }

    fn parse_function_definition(line: &str) -> Option<(String, String, String)> {
        let eq_pos = line.find('=')?;
        let signature = line[..eq_pos].trim();
        let open = signature.find('(')?;
        let name = signature[..open].trim().to_string();
        let argument = signature[open + 1..].strip_suffix(')')?.trim().to_string();
        if name.is_empty() || argument.is_empty() {
            return None;
        }
        Some((name, argument, line[eq_pos + 1..].trim().to_string()))
    }

    fn parse_function_call(value: &str) -> Option<(String, String)> {
        let value = value.trim();
        let open = value.find('(')?;
        let name = value[..open].trim().to_string();
        let argument = value[open + 1..].strip_suffix(')')?;
        if name.is_empty() {
            return None;
        }
        Some((name, strip_quotes(argument.trim())?))
    }

    (config, items)
}

fn parse_items_json(command: &str) -> Vec<Item> {
    if command == "applist()" || command == "applist(noIcons)" {
        let no_icons = command == "applist(noIcons)";
        return parse_items_json_output(&generate_applist_json(no_icons));
    }

    let args = match split_command(command) {
        Some(args) if !args.is_empty() => args,
        _ => {
            eprintln!("ItemsJSON command is empty or invalid");
            return Vec::new();
        }
    };
    let output = match Command::new(&args[0]).args(&args[1..]).output() {
        Ok(output) => output,
        Err(error) => {
            eprintln!("failed to run ItemsJSON command: {}", error);
            return Vec::new();
        }
    };

    if !output.status.success() {
        eprintln!("ItemsJSON command failed with status {}", output.status);
        return Vec::new();
    }

    parse_items_json_output(&String::from_utf8_lossy(&output.stdout))
}

fn parse_items_json_output(output: &str) -> Vec<Item> {
    let json: serde_json::Value = match serde_json::from_str(output) {
        Ok(json) => json,
        Err(error) => {
            eprintln!("failed to parse ItemsJSON output: {}", error);
            return Vec::new();
        }
    };

    let Some(entries) = json.as_array() else {
        eprintln!("ItemsJSON output must be an array");
        return Vec::new();
    };

    entries
        .iter()
        .filter_map(|entry| {
            let name = entry.get("name")?.as_str()?;
            let command = entry.get("exec")?.as_str()?;
            Some(Item {
                name: name.to_string(),
                action: Action::Exec(command.to_string()),
                icon: entry
                    .get("icon")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string),
            })
        })
        .collect()
}

fn generate_applist_json(no_icons: bool) -> String {
    let home = env::var_os("HOME").map(PathBuf::from);
    let mut directories = vec![
        Path::new("/usr/share/applications").to_path_buf(),
        Path::new("/usr/local/share/applications").to_path_buf(),
    ];
    if let Some(home) = home {
        directories.push(home.join(".local/share/applications"));
    }

    let mut entries = Vec::new();
    for directory in directories {
        let Ok(files) = fs::read_dir(directory) else {
            continue;
        };
        for file in files.flatten() {
            let path = file.path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("desktop") {
                continue;
            }
            let Ok(contents) = fs::read_to_string(path) else {
                continue;
            };
            let fields = desktop_fields(&contents);
            if fields.get("Type").map(String::as_str).unwrap_or("Application") != "Application"
                || fields.get("NoDisplay").map(String::as_str) == Some("true")
                || fields.get("Hidden").map(String::as_str) == Some("true")
            {
                continue;
            }
            let (Some(name), Some(command)) = (fields.get("Name"), fields.get("Exec")) else {
                continue;
            };
            let mut entry = serde_json::json!({
                "name": name,
                "exec": desktop_exec(command),
            });
            if !no_icons {
                if let Some(icon) = fields.get("Icon") {
                    entry["icon"] = serde_json::Value::String(icon.clone());
                }
            }
            entries.push(entry);
        }
    }
    serde_json::to_string(&entries).unwrap_or_else(|_| "[]".to_string())
}

fn desktop_fields(contents: &str) -> HashMap<String, String> {
    contents
        .lines()
        .filter_map(|line| {
            let (key, value) = line.split_once('=')?;
            Some((key.to_string(), value.to_string()))
        })
        .collect()
}

fn desktop_exec(command: &str) -> String {
    command
        .split_whitespace()
        .filter(|part| !part.starts_with('%'))
        .collect::<Vec<_>>()
        .join(" ")
}

// parses:  "key" = "value or func(...)"
fn parse_kv(line: &str) -> Option<(String, String)> {
    let eq_pos = line.find('=')?;
    let (raw_key, raw_val) = line.split_at(eq_pos);
    let raw_val = &raw_val[1..]; // drop '='

    let key = strip_quotes(raw_key.trim())?;
    let val = raw_val.trim().to_string();

    Some((key, val))
}

fn strip_quotes(s: &str) -> Option<String> {
    let s = s.trim();
    if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        Some(s[1..s.len() - 1].to_string())
    } else {
        None
    }
}

// parses:  exec("some command")  ->  Some("some command")
fn parse_exec(value: &str) -> Option<String> {
    let value = value.trim();
    let inner = value.strip_prefix("exec(")?.strip_suffix(")")?;
    strip_quotes(inner.trim())
}

fn parse_submenu(value: &str) -> Option<String> {
    let value = value.trim();
    let inner = value.strip_prefix("submenu(")?.strip_suffix(")")?;
    strip_quotes(inner.trim())
}

fn run_launcher(config: &Config, items: &[Item]) -> Option<String> {
    let launcher = if config.menu.is_empty() {
        "rofi"
    } else {
        config.menu.as_str()
    };

    let input = items
        .iter()
        .map(|i| i.name.as_str())
        .collect::<Vec<_>>()
        .join("\n");

    let title = if config.title.is_empty() {
        "menupp"
    } else {
        config.title.as_str()
    };

    let output = match launcher {
        "rofi" => Command::new("rofi")
            .args(["-dmenu", "-i", "-p", title])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                use std::io::Write;
                child
                    .stdin
                    .as_mut()
                    .unwrap()
                    .write_all(input.as_bytes())?;
                child.wait_with_output()
            }),
        "fuzzel" => Command::new("fuzzel")
            .arg("--dmenu")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                use std::io::Write;
                child
                .stdin
                .as_mut()
                .unwrap()
                .write_all(input.as_bytes())?;
                child.wait_with_output()
            }),
        "wofi" => Command::new("wofi")
            .args(["--dmenu", "--prompt", title])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                use std::io::Write;
                child
                    .stdin
                    .as_mut()
                    .unwrap()
                    .write_all(input.as_bytes())?;
                child.wait_with_output()
            }),
        other => {
            eprintln!("unsupported launcher: {}", other);
            exit(1);
        }
    };

    match output {
        Ok(out) => {
            let choice = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if choice.is_empty() {
                None
            } else {
                Some(choice)
            }
        }
        Err(e) => {
            eprintln!("failed to run launcher: {}", e);
            None
        }
    }
}

fn run_command(cmd: &str) {
    let args = match split_command(cmd) {
        Some(args) if !args.is_empty() => args,
        _ => {
            eprintln!("command is empty or invalid: {}", cmd);
            return;
        }
    };
    let status = Command::new(&args[0]).args(&args[1..]).status();

    if let Err(e) = status {
        eprintln!("failed to run command: {}", e);
    }
}

fn split_command(command: &str) -> Option<Vec<String>> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut escaped = false;

    for character in command.chars() {
        if escaped {
            current.push(character);
            escaped = false;
        } else if character == '\\' && quote != Some('\'') {
            escaped = true;
        } else if let Some(active_quote) = quote {
            if character == active_quote {
                quote = None;
            } else {
                current.push(character);
            }
        } else if character == '\'' || character == '"' {
            quote = Some(character);
        } else if character.is_whitespace() {
            if !current.is_empty() {
                args.push(std::mem::take(&mut current));
            }
        } else {
            current.push(character);
        }
    }

    if escaped || quote.is_some() {
        return None;
    }
    if !current.is_empty() {
        args.push(current);
    }
    Some(args)
}

#[cfg(test)]
mod tests {
    use super::{parse, parse_items_json, parse_named_section, Action};

    #[test]
    fn expands_function_argument_in_exec_command() {
        let content = r#"
Functions:
kitten(a) = exec("kitty -c ${a}")

Items:
"About" = kitten("fastfetch")
"#;

        let (_, items) = parse(content);

        assert_eq!(items.len(), 1);
        match &items[0].action {
            Action::Exec(command) => assert_eq!(command, "kitty -c fastfetch"),
            _ => panic!("expected an exec action"),
        }
    }

    #[test]
    fn parses_items_json_output() {
        let items = parse_items_json(
            r#"printf '%s' '[{"name":"Firefox","exec":"firefox"},{"name":"Discord","exec":"discord"}]'"#,
        );

        assert_eq!(items.len(), 2);
        assert!(matches!(&items[0].action, Action::Exec(command) if command == "firefox"));
        assert_eq!(items[1].name, "Discord");
    }

    #[test]
    fn parses_inline_named_submenu() {
        let content = r#"
Config:
"menu" = "rofi"
Items:
"About" = submenu("*applist")

*applist:
Config:
"title" = "Launch"
Items:
"Firefox" = exec("firefox")
"#;

        let (config, items) = parse(content);
        assert_eq!(items.len(), 1);
        assert!(matches!(items[0].action, Action::Submenu(ref name) if name == "*applist"));

        let (submenu_config, submenu_items) = parse_named_section(content, "applist");
        assert_eq!(config.menu, "rofi");
        assert_eq!(submenu_config.title, "Launch");
        assert_eq!(submenu_items.len(), 1);
    }

    #[test]
    fn parses_no_result_command() {
        let content = r#"
No-result-found:
"Search google for ${term}" = exec("./search ${term}")
"#;

        let (config, items) = parse(content);
        assert!(items.is_empty());
        assert_eq!(
            config.no_result.as_ref().map(|(_, command)| command.as_str()),
            Some("./search ${term}")
        );
    }
}
