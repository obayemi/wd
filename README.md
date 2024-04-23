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

That will install the wd binary (named `wdbin`), then we need to setup your shell.

you will also need to create wd's xdg data directory with
```
mkdir ~/.local/share/wd
```
(maybe one day I'll add a thing to auto-create it, but who knows)

## Bash / Zsh

Put this somewhere it will be sourced (bashrc/zshrc or some script that will be
sourced from it)

```sh
function wd () {
  local target
  target=$("${WDBIN:-"wdbin"}" complete "$@")
  if [ $? -eq 0 ]; then
    builtin cd "$target"
  fi
}

alias cd=wd  # or not if you want to keep your `cd` working as if it were the 80s
```

## Special snowflakes (fish users)

run

```
function wd
  set target (wdbin complete "$argv")

  if test "$status" -eq 0
    builtin cd "$target"
  end
end

funcsave wd
```

And then, the part where you don't want to lose the crapton of feature that are
in fish's cd that are not actually in the builtin, like `cd -`, `cd .`, and
some part of the pwd history stack

```
function cd --description 'Change directory'
    set -l MAX_DIR_HIST 25

    if test (count $argv) -gt 1
        printf "%s\n" (_ "Too many args for cd command")
        return 1
    end

    # Skip history in subshells.
    if status --is-command-substitution
        builtin cd $argv
        return $status
    end

    # Avoid set completions.
    set -l previous $PWD

    if test "$argv" = "-"
        if test "$__fish_cd_direction" = "next"
            nextd
        else
            prevd
        end
        return $status
    end

    # allow explicit "cd ." if the mount-point became stale in the meantime
    if test "$argv" = "."
        cd "$PWD"
        return $status
    end

    if test (count $argv) -eq 0
      cd $HOME
      return $status
    end


    wd $argv  # notice how that's the one and only Thing that we actually want to change
    set -l cd_status $status

    if test $cd_status -eq 0 -a "$PWD" != "$previous"
        set -q dirprev
        or set -l dirprev
        set -q dirprev[$MAX_DIR_HIST]
        and set -e dirprev[1]

        # If dirprev, dirnext, __fish_cd_direction
        # are set as universal variables, honor their scope.

        set -U -q dirprev
        and set -U -a dirprev $previous
        or set -g -a dirprev $previous

        set -U -q dirnext
        and set -U -e dirnext
        or set -e dirnext

        set -U -q __fish_cd_direction
        and set -U __fish_cd_direction prev
        or set -g __fish_cd_direction prev
    end

    return $cd_status
end

funcsave cd
```

