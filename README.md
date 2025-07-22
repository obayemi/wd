# wd
Your entire filesystem have never been so close.

`cd` is boring and outdated, who in 2020 want to make efforts to remember where
your stuff is. So replace the boring `cd` buitin of your shell with `wd`, and
you can now just use `cd` to go to:
- any local directory or direcory with absolute or relative path 
- any previously visited directory with its dir name
- any previously visited directory you don't remember the name of or are too
  drunk to type correctly

#example:
with a previously visited `$HOME/dev/demo/boring_directory` holding your most
precious files, from **wherever in your filesystem**, these commands will have
the same effect.

```sh
cd ~/dev/demo/boring_directory
cd boring_directory
cd bordire
cd boringdemo
cd boringdiaaegrjh
```

# Installation:

```
cargo install --git https://github.com/obayemi/wd
```

That will install the wd binary (named `wdbin`). Now you need to setup your shell.

## Shell Setup

Use the built-in `init` command to get the appropriate setup for your shell:

### Bash
```bash
echo 'eval "$(wdbin init bash)"' >> ~/.bashrc
source ~/.bashrc
```

### Zsh
```zsh
echo 'eval "$(wdbin init zsh)"' >> ~/.zshrc
source ~/.zshrc
```

### Fish
```fish
# Add to your Fish config file
wdbin init fish >> ~/.config/fish/config.fish
# Then reload your config or restart Fish
```