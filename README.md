# `aw-time-limit`

Currently this is only tested on Linux, but theoretically compatible with macOS and Windows too.

To build this, you will need Rust ([rustup.rs](https://rustup.rs)) as well as libssl-dev on Linux (not sure about the packages on macOS or Windows though). If everything is set up properly, then just run `cargo install --path .` at the root of this repository.

You will have to manually configure your OS to run `awtl daemon` on boot, which can be done in KDE using the Startup and Shutdown > Autostart page in the system settings, or by creating a custom `.desktop` file in Gnome.

The default daily time limit is 7h30m and is currently hardcoded. However you can extend your time limit for a day by running `awtl extend <time>` where `<time>` is a number followed by an `h` for hours, `m` for minutes, or `s` for seconds. If you already set an extension for the day previously, this command will overwrite that extension with the new time you set.

There is one more subcommand, `awtl status`, which will show you general information about your tracked time and time limits for the day.

Also if you want to get really advanced, `awtl extend` is just a command to write a `~/.time-limit-extension` file. It's stored in the format `MM/DD/YYYY <seconds>`, and can store extensions for multiple days separated on different lines. However `awtl extend <time>` is very naive and will currently overwrite this file entirely each time.
