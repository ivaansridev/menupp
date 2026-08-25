# menupp (Menu++) v0.1

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

License: MIT
