# menupp (Menu++) v0.2

A simple menu for rofi and fuzzel

Example:

```mpp
Config:
    "menu" = "rofi" # also fuzzel
    "title" = "Menu"

Items:
    "About" = submenu("*applist")

*applist:

    Config:
        "menu" = "rofi"
        "title" = "Launch"

    ItemsJSON:
        applist()

```

Supported launchers:
- fuzzel
- rofi
- wofi

## Submenus

There are two kinds, and both push a `Back` entry onto the menu automatically.

**Another file.** The path is resolved next to the file that referenced it, and the
`.mpp` extension is optional, so `submenu("power")` reads `power.mpp`.

```mpp
Items:
    "Power" = submenu("power")
```

**A section in the same file.** Prefix the name with `*` and define the section
further down. Nothing is loaded from disk.

```mpp
Items:
    "Applications" = submenu("*apps")

*apps:
    Config:
        "title" = "Apps"

    Items:
        "Terminal" = exec("kitty")
```

Choosing `Back` returns to whichever menu opened the submenu.

## Commands

Commands run directly, **without a shell**, so pipes, redirects and `&&` do not
work. If you need those, put them in a script and call the script.

Arguments are split on whitespace with support for single and double quotes, so
`exec("sh -c 'ls | wc'")` works.

## Live values

`$("command")` runs a command and replaces itself with its output, which is handy
for status lines. Multi-line output is joined into one line, and a command that
fails simply renders as empty. An empty value (`""`) makes the entry display-only,
so selecting it does nothing.

```mpp
Items:
    "Uptime: $("uptime -p")" = ""
```

## No-result-found

If what you type matches none of the items, the menu shows an empty screen. Pressing
enter there runs the fallback command straight away -- no confirmation prompt and no
second menu, so it works with launchers that only allow one instance at a time.

Declare it as a literal `no-result-found` item. It is not shown as a normal entry:

```
Items:
    "Apps" = exec("rofi -show drun")
    "no-result-found" = exec("xdg-open https://www.google.com/search?q=${term}")
```

A `No-result-found:` block works too, if you prefer a separate section:

```
No-result-found:
    "Search google for ${term}" = exec("./your-happy-script ${term}")
```

`${term}` is replaced with the text you typed and quoted, so a term containing spaces
arrives as a single argument. The label in the block form is decorative.

License: MIT