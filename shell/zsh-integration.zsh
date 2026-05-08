autoload -Uz add-zsh-hook

_tide_escape_osc() {
  printf '%s' "$1" | command od -An -tx1 -v | command tr -d ' \n'
}

_tide_preexec() {
  local cmd="$1"
  cmd="$(_tide_escape_osc "$cmd")"
  printf '\033]777;block_start;cmd=hex:%s\a' "$cmd"
}

_tide_precmd() {
  local ec=$?
  local cwd="$PWD"
  cwd="$(_tide_escape_osc "$cwd")"
  printf '\033]777;block_end;exit=%d;cwd=hex:%s\a' "$ec" "$cwd"
}

add-zsh-hook preexec _tide_preexec
add-zsh-hook precmd _tide_precmd
