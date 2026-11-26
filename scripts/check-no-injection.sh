#!/usr/bin/env bash
# Fails if the source tree uses an API that could load code into, or write
# into, another process, or if a crate is built as a DLL.
#
# Allowed exceptions, both documented in docs/architecture.md:
#   - SetWindowsHookExW only with WH_MOUSE_LL (the callback runs in Bento
#     Desk's own thread, no module is mapped into other processes);
#   - SetWinEventHook only with WINEVENT_OUTOFCONTEXT.
set -euo pipefail

cd "$(dirname "$0")/.."
status=0

forbidden='VirtualAllocEx|WriteProcessMemory|CreateRemoteThread|NtCreateThreadEx|RtlCreateUserThread|QueueUserAPC|NtQueueApcThread|SetThreadContext|DllRegisterServer|DllGetClassObject|DllCanUnloadNow|LoadLibrary|DetourAttach|MH_CreateHook|AppInit_DLLs|WINEVENT_INCONTEXT'
if grep -rnE "$forbidden" --include='*.rs' crates apps; then
  echo "error: forbidden API found" >&2
  status=1
fi

# Every SetWindowsHookEx call must install WH_MOUSE_LL.
while IFS= read -r line; do
  if ! grep -q 'WH_MOUSE_LL' <<<"$line"; then
    echo "error: hook other than WH_MOUSE_LL: $line" >&2
    status=1
  fi
done < <(grep -rn 'SetWindowsHookEx[AW]\?(' --include='*.rs' crates apps || true)

# Every SetWinEventHook call must be out of context.
if grep -rln 'SetWinEventHook(' --include='*.rs' crates apps \
  | xargs -r grep -L 'WINEVENT_OUTOFCONTEXT' | grep .; then
  echo "error: SetWinEventHook without WINEVENT_OUTOFCONTEXT" >&2
  status=1
fi

# No library crate may be built as a DLL.
if grep -rnE 'crate-type|cdylib|dylib' --include='Cargo.toml' . ; then
  echo "error: a crate is configured as a dynamic library" >&2
  status=1
fi

if [ "$status" -eq 0 ]; then
  echo "no injection API found"
fi
exit "$status"
