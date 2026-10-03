# v0.2
+ `sh()` runs a command through your `$SHELL`, so pipes, redirects, globs and && work
+ `$("cmd")` runs a command and inlines its output, for live status lines
+ `no-result-found` item: runs when typed text matches nothing, with `${term}` set
+ `submenu("power")` no longer needs the `.mpp` extension
+ `Functions:` entries can be built with `exec()` or `sh()`
+ an empty value `""` marks a display-only entry that does nothing when picked
+ trailing `#` comments on a config line now work, as the README always claimed
+ `${term}` is quoted, so a term with spaces arrives as one argument
+ `backtext` config option
+ submenu output is flattened to one line so it cannot inject fake menu entries

# v0.1
+ .mpp File extension
+ Fuzzel and rofi support
+ New config option "title"
A lot more features (since this is the first release)