use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, exit};

/// Literal item name that registers the no-result-found fallback command.
const NO_RESULT_ITEM: &str = "no-result-found";

#[derive(Debug)]
struct Config {
    menu: String,
    title: String,
    backtext: String,
    back: bool,
    no_result: Option<(String, bool)>,
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

#[derive(Clone, Debug)]
struct Item {
    name: String,
    action: Action,
}

#[derive(Debug)]
struct Function {
    argument: String,
    command: String,
    shell: bool,
}

#[derive(Clone, Debug)]
enum Action {
    Exec(String),
    /// Runs through the user's shell, so pipes and redirects work.
    Shell(String),
    Submenu(String),
    Back,
    /// Display-only entry: it shows up in the menu but does nothing when picked.
    None,
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
            },
        );
    }
    if items.is_empty() {
        eprintln!("no items found in {}", path.display());
        exit(1);
    }

    loop {
        let Some(choice) = run_launcher(&config, &items) else {
            return;
        };

        // Nothing matched what was typed. If a fallback is configured, run it
        // straight away with the typed text as ${term} -- no second prompt.
        let Some(item) = items.iter().find(|i| i.name == choice) else {
            if let Some((command, shell)) = &config.no_result {
                let command = command.replace("${term}", &shell_quote(&choice));
                if *shell {
                    run_shell(&command);
                } else {
                    run_command(&command);
                }
            }
            return;
        };

        match &item.action {
            Action::Exec(command) => {
                run_command(command.as_str());
                return;
            }
            Action::Shell(command) => {
                run_shell(command.as_str());
                return;
            }
            Action::Submenu(submenu) => {
                if let Some(section) = submenu.strip_prefix('*') {
                    run_menu_content(path, content, true, Some(section));
                } else {
                    run_menu(&resolve_submenu(path, submenu), true);
                }
            }

            Action::Back => return,
            Action::None => return,
        }
    }
}

// Replaces every $("command") with the trimmed stdout of that command, so a
// line like "Uptime: $("uptime")" renders live status in the menu.
fn expand_substitutions(input: &str) -> String {
    let mut out = String::new();
    let mut rest = input;

    while let Some(start) = rest.find("$(") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let (body, quoted) = match after.strip_prefix('"') {
            Some(body) => (body, true),
            None => (after, false),
        };
        let Some((end, skip)) = find_substitution_end(body, quoted) else {
            // Unterminated: keep the text as-is rather than eating the rest.
            out.push_str(&rest[start..]);
            return out;
        };
        out.push_str(&capture_output(body[..end].trim()));
        rest = &body[end + skip..];
    }

    out.push_str(rest);
    out
}

// Locates where the command text ends, as (index, chars_to_skip). A quoted
// $("...") ends at `")`; a bare $(...) ends at the first `)`.
fn find_substitution_end(s: &str, quoted: bool) -> Option<(usize, usize)> {
    let mut in_quote = false;
    let mut chars = s.char_indices().peekable();

    while let Some((i, c)) = chars.next() {
        if c == '"' {
            if quoted && matches!(chars.peek(), Some((_, ')'))) {
                return Some((i, 2));
            }
            in_quote = !in_quote;
        } else if c == ')' && !quoted && !in_quote {
            return Some((i, 1));
        }
    }

    None
}

// Drops a trailing `#` comment. The `#` must be outside quotes and start the
// line or follow whitespace, so paths like foo#bar and colours survive.
fn strip_comment(line: &str) -> &str {
    let mut in_quote = false;
    let mut i = 0;

    while i < line.len() {
        let rest = &line[i..];
        let c = rest.chars().next().unwrap();

        match c {
            '"' => {
                in_quote = !in_quote;
                i += c.len_utf8();
            }
            '$' if rest[1..].starts_with('(') => {
                let after = &rest[2..];
                let (body, quoted) = match after.strip_prefix('"') {
                    Some(body) => (body, true),
                    None => (after, false),
                };
                match find_substitution_end(body, quoted) {
                    Some((end, skip)) => {
                        i += 2 + end + skip;
                        continue;
                    }
                    None => i += c.len_utf8(),
                }
            }
            '#' if !in_quote
                && line[..i]
                    .chars()
                    .next_back()
                    .is_none_or(char::is_whitespace) =>
            {
                return &line[..i];
            }
            _ => i += c.len_utf8(),
        }
    }

    line
}

// Finds the `=` that separates key from value, skipping any that sit inside
// quotes or inside a $("...") substitution such as $("echo a=b").
fn find_assignment(line: &str) -> Option<usize> {
    let mut i = 0;
    let mut in_quote = false;

    while i < line.len() {
        let rest = &line[i..];
        let c = rest.chars().next()?;
        match c {
            '"' => {
                in_quote = !in_quote;
                i += c.len_utf8();
            }
            // Handled regardless of quote state: $("...") nests its own quotes
            // inside the surrounding string literal.
            '$' if rest[1..].starts_with('(') => {
                let after = &rest[2..];
                let (body, quoted) = match after.strip_prefix('"') {
                    Some(body) => (body, true),
                    None => (after, false),
                };
                match find_substitution_end(body, quoted) {
                    Some((end, skip)) => {
                        i += 2 + end + skip;
                        continue;
                    }
                    None => i += c.len_utf8(),
                }
            }
            '=' if !in_quote => return Some(i),
            _ => i += c.len_utf8(),
        }
    }

    None
}

fn capture_output(command: &str) -> String {
    let shell = user_shell();

    match Command::new(&shell).arg("-c").arg(command).output() {
        Ok(output) if output.status.success() => {
            // Flatten to a single line: the launcher's input is line-delimited,
            // so multi-line output would otherwise become phantom menu entries.
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .collect::<Vec<_>>()
                .join(" ")
        }
        _ => String::new(),
    }
}

// Resolves a submenu target next to the file that referenced it. The .mpp
// extension is optional, so submenu("power") reads power.mpp.
fn resolve_submenu(from: &Path, submenu: &str) -> PathBuf {
    let mut path = from
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(submenu);
    if path.extension().is_none() {
        path.set_extension("mpp");
    }
    path
}

fn parse_named_section(content: &str, name: &str) -> (Config, Vec<Item>) {
    let mut section_lines = Vec::new();
    let mut in_section = false;

    for raw_line in content.lines() {
        let line = strip_comment(raw_line.trim());
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
        let line = strip_comment(raw_line.trim()).trim();

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
                        "back"
                            if strip_quotes(&value).as_deref() == Some("false")
                                || value.trim() == "false" =>
                        {
                            config.back = false;
                        }
                        _ => {}
                    }
                }
            }
            Block::Functions => {
                if let Some((name, argument, value)) = parse_function_definition(line)
                    && let Some((command, shell)) = parse_command_action(&value)
                {
                    functions.insert(
                        name,
                        Function {
                            argument,
                            command,
                            shell,
                        },
                    );
                }
            }
            Block::Items => {
                if let Some((key, value)) = parse_kv(line) {
                    // A literal "no-result-found" item declares the fallback
                    // command instead of adding a visible entry to the menu.
                    if key.eq_ignore_ascii_case(NO_RESULT_ITEM) {
                        if let Some(resolved) = parse_command_action(&value) {
                            config.no_result = Some(resolved);
                        }
                        continue;
                    }
                    let action = parse_action(&value, &functions)
                        // An empty value marks a display-only entry, e.g. a
                        // live status line: "Uptime: $("uptime")" = ""
                        .or_else(|| {
                            value
                                .trim_matches('"')
                                .trim()
                                .is_empty()
                                .then_some(Action::None)
                        });
                    if let Some(action) = action {
                        items.push(Item { name: key, action });
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
                if let Some((_, value)) = parse_kv(line)
                    && let Some(resolved) = parse_command_action(&value)
                {
                    config.no_result = Some(resolved);
                }
            }
            Block::None => {}
        }
    }

    fn parse_action(value: &str, functions: &HashMap<String, Function>) -> Option<Action> {
        if let Some((command, shell)) = parse_command_action(value) {
            return Some(if shell {
                Action::Shell(command)
            } else {
                Action::Exec(command)
            });
        }
        if let Some(submenu) = parse_submenu(value) {
            return Some(Action::Submenu(submenu));
        }
        let (name, argument) = parse_function_call(value)?;
        let function = functions.get(&name)?;
        let command = function
            .command
            .replace(&format!("${{{}}}", function.argument), &argument);
        Some(if function.shell {
            Action::Shell(command)
        } else {
            Action::Exec(command)
        })
    }

    fn parse_function_definition(line: &str) -> Option<(String, String, String)> {
        let eq_pos = find_assignment(line)?;
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
        return parse_items_json_output(&generate_applist_json());
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
            })
        })
        .collect()
}

fn generate_applist_json() -> String {
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
            if fields
                .get("Type")
                .map(String::as_str)
                .unwrap_or("Application")
                != "Application"
                || fields.get("NoDisplay").map(String::as_str) == Some("true")
                || fields.get("Hidden").map(String::as_str) == Some("true")
            {
                continue;
            }
            let (Some(name), Some(command)) = (fields.get("Name"), fields.get("Exec")) else {
                continue;
            };
            let entry = serde_json::json!({
                "name": name,
                "exec": desktop_exec(command),
            });
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
    let eq_pos = find_assignment(line)?;
    let (raw_key, raw_val) = line.split_at(eq_pos);
    let raw_val = &raw_val[1..]; // drop '='

    let key = expand_substitutions(&strip_quotes(raw_key.trim())?);
    let val = expand_substitutions(raw_val.trim());

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

// parses:  sh("some | command")  ->  Some("some | command")
fn parse_sh(value: &str) -> Option<String> {
    let value = value.trim();
    let inner = value.strip_prefix("sh(")?.strip_suffix(")")?;
    strip_quotes(inner.trim())
}

// Resolves exec("...") or sh("...") into (command, run_in_shell).
fn parse_command_action(value: &str) -> Option<(String, bool)> {
    if let Some(command) = parse_exec(value) {
        return Some((command, false));
    }
    parse_sh(value).map(|command| (command, true))
}

fn parse_submenu(value: &str) -> Option<String> {
    let value = value.trim();
    let inner = value.strip_prefix("submenu(")?.strip_suffix(")")?;
    strip_quotes(inner.trim())
}

// Wraps a term in single quotes so split_command keeps it as one argument.
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
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
                child.stdin.as_mut().unwrap().write_all(input.as_bytes())?;
                child.wait_with_output()
            }),
        "fuzzel" => Command::new("fuzzel")
            .arg("--dmenu")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                use std::io::Write;
                child.stdin.as_mut().unwrap().write_all(input.as_bytes())?;
                child.wait_with_output()
            }),
        "wofi" => Command::new("wofi")
            .args(["--dmenu", "--prompt", title])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                use std::io::Write;
                child.stdin.as_mut().unwrap().write_all(input.as_bytes())?;
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

// The user's login shell, so sh() and $() behave the way their terminal does.
fn user_shell() -> String {
    resolve_shell(env::var("SHELL").ok().as_deref())
}

fn resolve_shell(shell: Option<&str>) -> String {
    shell
        .filter(|shell| !shell.is_empty() && Path::new(shell).is_file())
        .unwrap_or("/bin/sh")
        .to_string()
}

// Runs a command through the user's shell, which is what makes pipes,
// redirects, globs and && work. The command is passed as a single -c
// argument, so no extra escaping is needed here.
fn run_shell(cmd: &str) {
    let shell = user_shell();
    let status = Command::new(&shell).arg("-c").arg(cmd).status();

    if let Err(e) = status {
        eprintln!("failed to run command in {}: {}", shell, e);
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
    use super::{
        Action, expand_substitutions, find_assignment, parse, parse_items_json,
        parse_named_section, resolve_shell, resolve_submenu, shell_quote, split_command,
    };
    use std::path::Path;

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
            config.no_result.as_ref().map(|(c, s)| (c.as_str(), *s)),
            Some(("./search ${term}", false))
        );
    }

    #[test]
    fn fallback_term_stays_a_single_argument() {
        let expanded = "./search ${term}".replace("${term}", &shell_quote("hello world"));

        assert_eq!(expanded, "./search 'hello world'");
        assert_eq!(
            split_command(&expanded).unwrap(),
            vec!["./search".to_string(), "hello world".to_string()]
        );
    }

    #[test]
    fn fallback_term_survives_embedded_single_quote() {
        let expanded = "./search ${term}".replace("${term}", &shell_quote("it's here"));

        assert_eq!(
            split_command(&expanded).unwrap(),
            vec!["./search".to_string(), "it's here".to_string()]
        );
    }

    #[test]
    fn command_substitution_runs_and_replaces_itself() {
        assert_eq!(expand_substitutions(r#"a $("echo hi") b"#), "a hi b");
        assert_eq!(
            expand_substitutions(r#"$("echo one") $("echo two")"#),
            "one two"
        );
        assert_eq!(expand_substitutions(r#"$(echo bare)"#), "bare");
        // Multi-line output is flattened so it cannot inject fake menu entries.
        assert_eq!(expand_substitutions(r#"$("printf 'a\nb\n'")"#), "a b");
    }

    #[test]
    fn command_substitution_keeps_equals_sign_inside() {
        // The `=` must not be mistaken for the key/value separator.
        assert_eq!(find_assignment(r#"x = $("echo a=b")"#), Some(2));
        assert_eq!(expand_substitutions(r#"$("echo a=b")"#), "a=b");
    }

    #[test]
    fn command_substitution_survives_a_failing_command() {
        assert_eq!(expand_substitutions(r#"pre $("exit 3") post"#), "pre  post");
    }

    #[test]
    fn unterminated_substitution_is_left_alone() {
        assert_eq!(expand_substitutions(r#"$("oops"#), r#"$("oops"#);
    }

    #[test]
    fn display_only_item_from_empty_value() {
        let content = r#"
Items:
"Uptime: $("echo 5m")" = ""
"#;

        let (_, items) = parse(content);

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "Uptime: 5m");
        assert!(matches!(items[0].action, Action::None));
    }

    #[test]
    fn sh_item_parses_as_a_shell_action() {
        let content = r#"
Items:
"Pipes" = sh("ls | wc -l")
"Plain" = exec("ls")
"#;

        let (_, items) = parse(content);

        assert!(matches!(&items[0].action, Action::Shell(c) if c == "ls | wc -l"));
        assert!(matches!(&items[1].action, Action::Exec(c) if c == "ls"));
    }

    #[test]
    fn sh_function_keeps_its_shell_flag() {
        let content = r#"
Functions:
piped(a) = sh("echo ${a} | tr a-z A-Z")

Items:
"Loud" = piped("hey")
"#;

        let (_, items) = parse(content);

        assert!(matches!(&items[0].action, Action::Shell(c) if c == "echo hey | tr a-z A-Z"));
    }

    #[test]
    fn user_shell_prefers_an_existing_shell() {
        assert_eq!(resolve_shell(Some("/usr/bin/zsh")), "/usr/bin/zsh");
        // Bogus or missing values fall back rather than failing at launch time.
        assert_eq!(resolve_shell(Some("/nope/not-a-shell")), "/bin/sh");
        assert_eq!(resolve_shell(Some("")), "/bin/sh");
        assert_eq!(resolve_shell(None), "/bin/sh");
    }

    #[test]
    fn submenu_extension_is_optional() {
        let from = Path::new("/home/user/menus/main.mpp");

        assert_eq!(
            resolve_submenu(from, "power"),
            Path::new("/home/user/menus/power.mpp")
        );
        assert_eq!(
            resolve_submenu(from, "power.mpp"),
            Path::new("/home/user/menus/power.mpp")
        );
        assert_eq!(
            resolve_submenu(from, "nested/power"),
            Path::new("/home/user/menus/nested/power.mpp")
        );
    }

    #[test]
    fn literal_no_result_item_is_hidden_from_the_menu() {
        let content = r#"
Items:
"Apps" = exec("rofi -show drun")
"no-result-found" = exec("xdg-open https://www.google.com/search?q=${term}")
"#;

        let (config, items) = parse(content);

        assert_eq!(
            config
                .no_result
                .as_ref()
                .map(|(command, shell)| (command.as_str(), *shell)),
            Some(("xdg-open https://www.google.com/search?q=${term}", false))
        );
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "Apps");
    }

    #[test]
    fn fallback_label_renders_term() {
        // The label is decorative; only the command survives parsing.
        let content = r#"
No-result-found:
"Search google for ${term}" = exec("./search ${term}")
"#;
        let (config, items) = parse(content);
        assert!(items.is_empty());
        assert_eq!(
            config.no_result.as_ref().map(|(c, s)| (c.as_str(), *s)),
            Some(("./search ${term}", false))
        );
    }
}
