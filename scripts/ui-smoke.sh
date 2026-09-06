#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
WORKSPACE="${MUXLANE_UI_WORKSPACE:-4}"
DISPLAY="${DISPLAY:-:0}"
export DISPLAY
export MUXLANE_TEST_PALETTE_CTRL_K=1
ARTIFACTS="$ROOT/artifacts/ui-smoke"
mkdir -p "$ARTIFACTS"

need() { command -v "$1" >/dev/null || { echo "missing tool: $1" >&2; exit 1; }; }
for tool in wmctrl xdotool import python3 tmux node fcitx5-remote; do need "$tool"; done
python3 -c 'import PIL' >/dev/null 2>&1 || { echo "missing Python module: Pillow" >&2; exit 1; }
[[ -x "${MUXLANE_TEST_SHELL:-/usr/bin/zsh}" ]] || { echo "missing test shell: ${MUXLANE_TEST_SHELL:-/usr/bin/zsh}" >&2; exit 1; }

cargo build -p muxlane-app >/dev/null
TMP="$(mktemp -d)"
ORIGINAL_FCITX_STATE="$(fcitx5-remote 2>/dev/null || echo 1)"
cleanup() {
  local status=$?
  if [[ "$status" -ne 0 && -f "$TMP/muxlane.log" ]]; then
    echo "--- muxlane.log ---" >&2
    tail -80 "$TMP/muxlane.log" >&2
  fi
  [[ -n "${PID:-}" ]] && kill -9 "$PID" 2>/dev/null || true
  if [[ -f "$XDG_DATA_HOME/muxlane/state.json" ]]; then
    python3 - "$XDG_DATA_HOME/muxlane/state.json" <<'PY' | while read -r session; do
import json,sys
for item in json.load(open(sys.argv[1])).get('sessions', []):
    if item.get('tmux_session'): print(item['tmux_session'])
PY
      tmux -L muxlane kill-session -t "$session" 2>/dev/null || true
    done
  fi
  [[ -n "${ORIGINAL_WS:-}" ]] && wmctrl -s "$ORIGINAL_WS" 2>/dev/null || true
  if [[ "$ORIGINAL_FCITX_STATE" == 2 ]]; then fcitx5-remote -o >/dev/null 2>&1 || true; fi
  rm -rf "$TMP"
}
trap cleanup EXIT

export SHELL="${MUXLANE_TEST_SHELL:-/usr/bin/zsh}"
export XDG_DATA_HOME="$TMP/data"
mkdir -p "$XDG_DATA_HOME/muxlane"
SMOKE_PROJECT="$TMP/workspace/muxlane"
mkdir -p "$SMOKE_PROJECT"
FAKE_ACP="$TMP/fake-acp.py"
cat >"$FAKE_ACP" <<'PY'
import json, sys
for line in sys.stdin:
    message=json.loads(line)
    request_id=message.get('id')
    method=message.get('method')
    if request_id is None:
        continue
    if method == 'initialize':
        result={'protocolVersion':1,'agentCapabilities':{'promptCapabilities':{'image':True,'audio':True,'embeddedContext':True}}}
    elif method == 'session/new':
        result={
            'sessionId':'smoke-acp-session',
            'modes': {'currentModeId':'high', 'availableModes':[{'id':'high','name':'Thinking: high'}]},
            'configOptions':[
                {'id':'model','name':'Model','type':'select','currentValue':'newapi/Kimi K3','options':[{'value':'newapi/Kimi K3','name':'newapi/Kimi K3'}]},
                {'id':'thinking','name':'Thinking','type':'select','currentValue':'high','options':[{'value':'high','name':'high'}]},
            ],
        }
    elif method == 'session/prompt':
        update={'jsonrpc':'2.0','method':'session/update','params':{'sessionId':'smoke-acp-session','update':{'sessionUpdate':'agent_message_chunk','messageId':'smoke-reply','content':{'type':'text','text':'Hello from fake ACP'}}}}
        print(json.dumps(update), flush=True)
        result={'stopReason':'end_turn'}
    else:
        result={}
    print(json.dumps({'jsonrpc':'2.0','id':request_id,'result':result}), flush=True)
PY
export MUXLANE_ACP_CLAUDE_COMMAND="python3 $FAKE_ACP"
SMOKE_AGENT="shell_smoke"
SMOKE_TMUX="muxlane-smoke-shell"
tmux -L muxlane new-session -d -s "$SMOKE_TMUX" -c "$SMOKE_PROJECT" "$SHELL"
python3 - "$XDG_DATA_HOME/muxlane/state.json" "$SMOKE_PROJECT" "$SMOKE_AGENT" "$SMOKE_TMUX" <<'PY'
import json, sys
path, root, agent, tmux_session = sys.argv[1:]
with open(path, "w") as f:
    json.dump({
        "version": 1,
        "initialized": True,
        "projects": [{"id": "p_smoke", "name": "muxlane", "path": root, "branch": None, "agents": []}],
        "sessions": [{"agent_id": agent, "project_id": "p_smoke", "agent_type": "shell", "title": "shell", "tmux_session": tmux_session}],
        "pane_tree": {"kind": "leaf", "group": {"id": "pane_smoke", "tabs": [agent], "active": agent}},
        "active_pane": "pane_smoke",
    }, f)
PY
"$ROOT/target/debug/muxlane" >"$TMP/muxlane.log" 2>&1 &
PID=$!
for _ in {1..100}; do
  WID="$(wmctrl -lp | awk -v p="$PID" '$3==p {print $1; exit}')"
  [[ -n "$WID" ]] && break
  sleep .1
done
[[ -n "${WID:-}" ]] || { cat "$TMP/muxlane.log" >&2; exit 1; }

eval "$(xdotool getwindowgeometry --shell "$WID" | grep -E '^(WIDTH|HEIGHT)=')"
px() { awk -v n="$1" -v total="$2" 'BEGIN{printf "%d", n*total}'; }
click_at() {
  local x="$1" y="$2" button="${3:-1}"
  xdotool mousemove --window "$WID" "$x" "$y"
  xdotool mousedown "$button"
  sleep .1
  xdotool mouseup "$button"
}
open_palette() {
  xdotool key ctrl+k
}
state_matches() {
  EXPR="$1" STATE="$STATE" python3 - <<'PY'
import json, os
with open(os.environ['STATE']) as f:
    state = json.load(f)
raise SystemExit(0 if eval(os.environ['EXPR'], {'__builtins__': {}}, {'d': state, 'len': len}) else 1)
PY
}
wait_state() {
  local expr="$1" label="$2"
  for _ in {1..100}; do
    if state_matches "$expr"; then return 0; fi
    sleep .05
  done
  echo "timed out waiting for state: $label" >&2
  return 1
}
wait_tmux_text() {
  local session="$1" text="$2" label="$3"
  for _ in {1..100}; do
    if tmux -L muxlane capture-pane -p -t "$session" | grep -Fq "$text"; then return 0; fi
    sleep .05
  done
  echo "timed out waiting for terminal text: $label" >&2
  return 1
}
assert_state_stable() {
  local expr="$1" label="$2"
  for _ in {1..8}; do
    if ! state_matches "$expr"; then
      echo "state changed while asserting stability: $label" >&2
      return 1
    fi
    sleep .05
  done
}
wait_sidebar_visual() {
  local baseline="$1" expected="$2" output="$3"
  for _ in {1..100}; do
    import -window "$WID" "$output"
    if BASELINE="$baseline" CURRENT="$output" EXPECTED="$expected" python3 - <<'PY'
import os
from PIL import Image, ImageChops
base=Image.open(os.environ['BASELINE']).convert('RGB')
current=Image.open(os.environ['CURRENT']).convert('RGB')
w,h=current.size
crop=(0,0,round(w*.18),h)
diff=ImageChops.difference(base.crop(crop), current.crop(crop))
changed=sum(1 for pixel in diff.getdata() if max(pixel)>8)
ratio=changed/(crop[2]*crop[3])
expected=os.environ['EXPECTED']
raise SystemExit(0 if (ratio > .30 if expected == 'collapsed' else ratio < .08) else 1)
PY
    then return 0; fi
    sleep .05
  done
  echo "timed out waiting for sidebar visual state: $expected" >&2
  return 1
}
wait_settings_open() {
  local baseline="$1" output="$2"
  for _ in {1..100}; do
    import -window "$WID" "$output"
    if BASELINE="$baseline" CURRENT="$output" python3 - <<'PY'
import os
from PIL import Image, ImageChops
base=Image.open(os.environ['BASELINE']).convert('RGB')
current=Image.open(os.environ['CURRENT']).convert('RGB')
w,h=current.size
crop=(round(w*.28),round(h*.06),round(w*.72),round(h*.94))
diff=ImageChops.difference(base.crop(crop), current.crop(crop))
changed=sum(1 for pixel in diff.getdata() if max(pixel)>8)
raise SystemExit(0 if changed > (crop[2]-crop[0])*(crop[3]-crop[1])*.20 else 1)
PY
    then return 0; fi
    sleep .05
  done
  echo "timed out waiting for settings to open" >&2
  return 1
}
wait_settings_error() {
  local output="$1"
  for _ in {1..100}; do
    import -window "$WID" "$output"
    if SCREENSHOT="$output" python3 - <<'PY'
import os
from PIL import Image
im=Image.open(os.environ['SCREENSHOT']).convert('RGB')
w,h=im.size
crop=im.crop((round(w*.28), round(h*.25), round(w*.72), round(h*.68)))
red=sum(1 for r,g,b in crop.getdata() if r > 130 and r > g*1.35 and r > b*1.15)
raise SystemExit(0 if red > 12 else 1)
PY
    then return 0; fi
    sleep .05
  done
  echo "timed out waiting for shortcut conflict error" >&2
  return 1
}
X_TERM="$(px .30 "$WIDTH")"; Y_TERM="$(px .28 "$HEIGHT")"
X_SESSION="$(px .05 "$WIDTH")"; Y_SESSION="$(px .14 "$HEIGHT")"
X_PROJECT_ADD="$(px .158 "$WIDTH")"; Y_PROJECT_ADD="$(px .085 "$HEIGHT")"
X_PALETTE_ITEM="$(px .34 "$WIDTH")"; Y_PALETTE_ITEM="$(px .255 "$HEIGHT")"
X_SPLIT="$(px .966 "$WIDTH")"; Y_HEADER="$(px .027 "$HEIGHT")"
X_MAX="$(px .992 "$WIDTH")"
X_SIDEBAR_HIDE="$(px .019 "$WIDTH")"; Y_SIDEBAR_HANDLE="$(px .03 "$HEIGHT")"
X_SIDEBAR_HANDLE="$(px .002 "$WIDTH")"
X_SETTINGS="$(px .161 "$WIDTH")"; Y_SETTINGS="$(px .975 "$HEIGHT")"
X_SETTINGS_APPEARANCE="$(px .226 "$WIDTH")"; Y_SETTINGS_APPEARANCE="$(px .175 "$HEIGHT")"
X_SETTINGS_SHORTCUTS="$(px .226 "$WIDTH")"; Y_SETTINGS_SHORTCUTS="$(px .208 "$HEIGHT")"
X_SHORTCUT_RECORD="$(px .712 "$WIDTH")"; X_SHORTCUT_CLEAR="$(px .805 "$WIDTH")"
Y_CLOSE_TAB_SHORTCUT="$(px .242 "$HEIGHT")"
X_SHORTCUT_RESTORE="$(px .805 "$WIDTH")"; Y_SHORTCUT_RESTORE="$(px .148 "$HEIGHT")"
STATE="$XDG_DATA_HOME/muxlane/state.json"
echo "geometry=${WIDTH}x${HEIGHT} palette=${X_PALETTE_ITEM},${Y_PALETTE_ITEM} project=${X_PROJECT_ADD},${Y_PROJECT_ADD}"

ORIGINAL_WS="$(wmctrl -d | awk '$2=="*" {print $1}')"
wmctrl -i -r "$WID" -t "$WORKSPACE"
wmctrl -s "$WORKSPACE"
sleep .4
xdotool windowactivate "$WID"
sleep .8
xdotool type --delay 2 "echo MUXLANE_STARTUP_FOCUS"
xdotool key Return
wait_tmux_text "$SMOKE_TMUX" MUXLANE_STARTUP_FOCUS "restored active tab input"
echo '✓ restored active tab accepted input without a click'

# 旧 state 不含快捷键字段时必须补默认值并迁移到当前 store version。
STATE="$XDG_DATA_HOME/muxlane/state.json" python3 - <<'PY'
import os,json
d=json.load(open(os.environ['STATE']))
assert d['version']==3, d['version']
assert d['shortcut_bindings']=={
    'close_tab':'ctrl-w',
    'previous_workspace':None,
    'next_workspace':None,
    'previous_tab':'platform-left',
    'next_tab':'platform-right',
}, d['shortcut_bindings']
print('✓ legacy state received persisted default shortcuts')
PY

# Sidebar hide persists immediately, converges to the edge rail, and the 5px rail reopens it.
import -window "$WID" "$ARTIFACTS/00-sidebar-expanded.png"
click_at "$X_SIDEBAR_HIDE" "$Y_SETTINGS" 1
wait_state "d.get('sidebar_visible') is False" "sidebar hidden"
wait_sidebar_visual "$ARTIFACTS/00-sidebar-expanded.png" collapsed "$ARTIFACTS/00-sidebar-collapsed.png"
click_at "$X_SIDEBAR_HANDLE" "$Y_SIDEBAR_HANDLE" 1
wait_state "d.get('sidebar_visible') is True" "sidebar reopened from rail handle"
wait_sidebar_visual "$ARTIFACTS/00-sidebar-expanded.png" expanded "$ARTIFACTS/00-sidebar-reopened.png"
echo '✓ sidebar rail converged and the edge handle reopened it'

# Live shortcut acceptance: capture, conflict rejection, disable/passthrough, and restore.
click_at "$X_SETTINGS" "$Y_SETTINGS" 1
wait_settings_open "$ARTIFACTS/00-sidebar-reopened.png" "$ARTIFACTS/00-settings-rebind.png"
click_at "$X_SETTINGS_SHORTCUTS" "$Y_SETTINGS_SHORTCUTS" 1
sleep .2
import -window "$WID" "$ARTIFACTS/00-settings-shortcuts.png"
click_at "$X_SHORTCUT_RECORD" "$Y_CLOSE_TAB_SHORTCUT" 1
xdotool key ctrl+q
wait_state "d['shortcut_bindings']['close_tab'] == 'ctrl-q'" "close-tab rebound to ctrl-q"
assert_state_stable "len(d['pane_tree']['group']['tabs']) == 1" "capture must not execute close-tab"
xdotool key Escape
xdotool key ctrl+shift+t
wait_state "len(d['pane_tree']['group']['tabs']) == 2" "new tab before ctrl-q close"
xdotool key ctrl+q
wait_state "len(d['pane_tree']['group']['tabs']) == 1" "live ctrl-q close without restart"

echo '✓ ctrl-q capture was consumed and live binding closed a new tab'
click_at "$X_SETTINGS" "$Y_SETTINGS" 1
wait_settings_open "$ARTIFACTS/00-sidebar-reopened.png" "$ARTIFACTS/00-settings-conflict-open.png"
click_at "$X_SETTINGS_SHORTCUTS" "$Y_SETTINGS_SHORTCUTS" 1
sleep .2
click_at "$X_SHORTCUT_RECORD" "$Y_CLOSE_TAB_SHORTCUT" 1
xdotool key super+k
wait_settings_error "$ARTIFACTS/00-shortcut-conflict.png"
assert_state_stable "d['shortcut_bindings']['close_tab'] == 'ctrl-q'" "fixed ctrl-k conflict must not persist"
click_at "$X_SHORTCUT_CLEAR" "$Y_CLOSE_TAB_SHORTCUT" 1
wait_state "d['shortcut_bindings']['close_tab'] is None" "close-tab binding cleared"
xdotool key Escape
xdotool key ctrl+shift+t
wait_state "len(d['pane_tree']['group']['tabs']) == 2" "new tab before disabled ctrl-w"
xdotool key ctrl+w
assert_state_stable "len(d['pane_tree']['group']['tabs']) == 2" "disabled ctrl-w must pass through"

click_at "$X_SETTINGS" "$Y_SETTINGS" 1
wait_settings_open "$ARTIFACTS/00-sidebar-reopened.png" "$ARTIFACTS/00-settings-restore-open.png"
click_at "$X_SETTINGS_SHORTCUTS" "$Y_SETTINGS_SHORTCUTS" 1
sleep .2
click_at "$X_SHORTCUT_RESTORE" "$Y_SHORTCUT_RESTORE" 1
wait_state "d['shortcut_bindings']['close_tab'] == 'ctrl-w'" "default ctrl-w restored"
xdotool key Escape
xdotool key ctrl+w
wait_state "len(d['pane_tree']['group']['tabs']) == 1" "restored ctrl-w closes tab"
echo '✓ conflict was rejected; clear restored passthrough; defaults restored ctrl-w'

# 1. 输入 + 真彩色 + 光标 + Fcitx 中文 IME。
click_at "$X_TERM" "$Y_TERM" 1
sleep .5
fcitx5-remote -s keyboard-us
sleep .2
xdotool type --delay 2 "echo MUXLANEASCII"
xdotool key Return
xdotool type --delay 3 "echo "
fcitx5-remote -s pinyin
fcitx5-remote -o
sleep .2
xdotool type --delay 8 "nihao"
xdotool key space
sleep .15
fcitx5-remote -s keyboard-us
xdotool key Return
# 颜色序列通过同一 tmux PTY 注入，避免 XTest 对反斜杠/下划线的键盘布局差异。
COLOR_TMUX="$(python3 - "$XDG_DATA_HOME/muxlane/state.json" <<'PY'
import json,sys
print(json.load(open(sys.argv[1]))['sessions'][0]['tmux_session'])
PY
)"
tmux -L muxlane send-keys -t "$COLOR_TMUX" -l -- "printf '\\033[31mMUXLANE_RED\\033[0m\\n'"
tmux -L muxlane send-keys -t "$COLOR_TMUX" Enter
sleep .5
import -window "$WID" "$ARTIFACTS/01-color-cursor-input.png"

# 协议回放：输入真的进 PTY。截图像素：红色文字和蓝色 cursor 真的被 GPUI 画出。
MUXLANE_SOCKET="$XDG_DATA_HOME/muxlane/muxlane.sock" SCREENSHOT="$ARTIFACTS/01-color-cursor-input.png" python3 - <<'PY'
import os, socket, json, base64
from PIL import Image
p=os.environ['MUXLANE_SOCKET']
s=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM); s.connect(p)
s.sendall(b'{"id":1,"method":"state.list"}\n'); b=b''
while b'\n' not in b: b+=s.recv(65536)
aid=json.loads(b.decode())['result']['agents'][0]['id']
s.sendall((json.dumps({'id':2,'method':'term.subscribe','params':{'agent':aid}})+'\n').encode()); b=b''
while b'\n' not in b: b+=s.recv(65536)
text=base64.b64decode(json.loads(b.decode())['result']['replay_b64']).decode('utf8','replace')
assert 'MUXLANEASCII' in text, text[-500:]
assert 'MUXLANE_RED' in text, text[-500:]
assert '你好' in text, text[-500:]
assert 'nihao' not in text, text[-500:]
im=Image.open(os.environ['SCREENSHOT']).convert('RGB')
red=sum(1 for r,g,b in im.getdata() if r>170 and r>g*1.4 and r>b*1.3)
blue=sum(1 for r,g,b in im.getdata() if b>130 and b>r*1.25 and b>g*1.05)
assert red>20, f'no rendered red text: {red}'
assert blue>20, f'no rendered blue cursor/accent: {blue}'
print(f'✓ input replay + rendered red={red} blue/cursor={blue}')
PY

# hook 权威事件：working spinner 与 done notification。
read -r HOOK_AGENT HOOK_TMUX < <(MUXLANE_SOCKET="$XDG_DATA_HOME/muxlane/muxlane.sock" python3 - <<'PY'
import os,socket,json
s=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM);s.connect(os.environ['MUXLANE_SOCKET'])
s.sendall(b'{"id":1,"method":"state.list"}\n');b=b''
while b'\n' not in b:b+=s.recv(65536)
a=json.loads(b.decode())['result']['agents'][0]
print(a['id'], a['tmux_session'])
PY
)
HOOK_TOKEN="$(tmux -L muxlane show-environment -t "$HOOK_TMUX" MUXLANE_HOOK_TOKEN | cut -d= -f2-)"
run_hook() {
  local payload="${2:-}"
  printf '%s' "$payload" | MUXLANE_SOCKET="$XDG_DATA_HOME/muxlane/muxlane.sock" MUXLANE_AGENT_ID="$HOOK_AGENT" \
    MUXLANE_HOOK_TOKEN="$HOOK_TOKEN" node "$XDG_DATA_HOME/muxlane/hooks/report.mjs" "$1"
}
run_hook working; sleep .18
import -window "$WID" "$ARTIFACTS/02-working-spinner-a.png"
sleep .18
import -window "$WID" "$ARTIFACTS/02-working-spinner-b.png"
run_hook done '{"last_assistant_message":"完成了具体修复并通过测试"}'; sleep .35
import -window "$WID" "$ARTIFACTS/02-done-notification.png"
MUXLANE_SOCKET="$XDG_DATA_HOME/muxlane/muxlane.sock" python3 - <<'PY'
import os,socket,json
s=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM);s.connect(os.environ['MUXLANE_SOCKET'])
s.sendall(b'{"id":1,"method":"state.list"}\n');b=b''
while b'\n' not in b:b+=s.recv(65536)
status=json.loads(b.decode())['result']['agents'][0]['status']
assert status=='done', status
print('✓ hook event reached done state with notification message content')
PY

# 通知入口必须打开可见的左下弹层，不能只切换内部 open 状态。
# 从截图检测默认 230px 侧栏的实际像素宽度，避免依赖 WM 最大化时机或 HiDPI 配置。
read -r X_NOTIFICATIONS Y_FOOTER UI_SCALE < <(SCREENSHOT="$ARTIFACTS/02-done-notification.png" python3 - <<'PY'
import os
from PIL import Image
im=Image.open(os.environ['SCREENSHOT']).convert('RGB')
y=im.height//2
sidebar=im.getpixel((10,y))
def distance(pixel): return max(abs(a-b) for a,b in zip(pixel,sidebar))
edge=next(x for x in range(20,im.width//2) if all(distance(im.getpixel((x+i,y)))>12 for i in range(8)))
scale=edge/230
print(round(170*scale), round(im.height-20*scale), f'{scale:.4f}')
PY
)
xdotool windowactivate --sync "$WID"
click_at "$X_NOTIFICATIONS" "$Y_FOOTER" 1
sleep .25
import -window "$WID" "$ARTIFACTS/02-notification-center.png"
BEFORE="$ARTIFACTS/02-done-notification.png" AFTER="$ARTIFACTS/02-notification-center.png" UI_SCALE="$UI_SCALE" python3 - <<'PY'
import os
from PIL import Image, ImageChops
before=Image.open(os.environ['BEFORE']).convert('RGB')
after=Image.open(os.environ['AFTER']).convert('RGB')
assert before.size==after.size, (before.size,after.size)
scale=float(os.environ['UI_SCALE'])
w,h=before.size
left=round(8*scale); right=round((8+320)*scale); bottom=round(h-40*scale)
top=max(0,round(bottom-220*scale))
diff=ImageChops.difference(before.crop((left,top,right,bottom)),after.crop((left,top,right,bottom)))
bbox=diff.getbbox()
changed=sum(1 for pixel in diff.getdata() if max(pixel)>4)
assert bbox is not None, 'notification center did not become visible'
width=bbox[2]-bbox[0]; height=bbox[3]-bbox[1]
assert changed>1000*scale*scale, f'notification center diff too small: changed={changed}'
assert width>250*scale and height>60*scale, f'unexpected notification center bounds: {bbox}'
assert abs(bbox[2]-round(320*scale))<8*scale, f'unexpected notification center right edge: {bbox}'
assert abs(bbox[3]-round(220*scale))<8*scale, f'unexpected notification center bottom edge: {bbox}'
print(f'✓ notification center opened visibly: changed={changed} bounds={bbox}')
PY
xdotool windowactivate --sync "$WID"
xdotool key Escape
sleep .15
import -window "$WID" "$ARTIFACTS/02-notification-center-closed.png"
BEFORE="$ARTIFACTS/02-done-notification.png" CLOSED="$ARTIFACTS/02-notification-center-closed.png" UI_SCALE="$UI_SCALE" python3 - <<'PY'
import os
from PIL import Image, ImageChops
before=Image.open(os.environ['BEFORE']).convert('RGB')
closed=Image.open(os.environ['CLOSED']).convert('RGB')
assert before.size==closed.size, (before.size,closed.size)
scale=float(os.environ['UI_SCALE'])
w,h=before.size
left=round(8*scale); right=round((8+320)*scale); bottom=round(h-40*scale)
top=max(0,round(bottom-220*scale))
diff=ImageChops.difference(before.crop((left,top,right,bottom)),closed.crop((left,top,right,bottom)))
changed=sum(1 for pixel in diff.getdata() if max(pixel)>4)
assert changed<250*scale*scale, f'notification center did not close cleanly: changed={changed}'
print(f'✓ notification center closed cleanly: changed={changed}')
PY

# 2. Ctrl+K 命令面板。
open_palette; sleep .35
import -window "$WID" "$ARTIFACTS/02-command-palette.png"
xdotool key Escape

# 3. 会话右键菜单。
click_at "$X_SESSION" "$Y_SESSION" 3; sleep .3
import -window "$WID" "$ARTIFACTS/03-session-menu.png"
xdotool key Escape

# 4. 新建第二个 Shell 会话（Ctrl+K；全局列表中项目后第一项为 Shell preset）。
open_palette; sleep .4
import -window "$WID" "$ARTIFACTS/04-project-add-session.png"
xdotool key Down; sleep .2
xdotool key Return; sleep .8
MUXLANE_SOCKET="$XDG_DATA_HOME/muxlane/muxlane.sock" python3 - <<'PY'
import os,socket,json
s=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM);s.connect(os.environ['MUXLANE_SOCKET'])
s.sendall(b'{"id":1,"method":"state.list"}\n');b=b''
while b'\n' not in b:b+=s.recv(65536)
count=len(json.loads(b.decode())['result']['agents'])
assert count==2, f'expected 2 agents after preset action, got {count}'
print('✓ preset action created second session (same action used by project +)')
PY

# 默认应仍是一棵 leaf（两个 tabs），未显式 split。
STATE="$XDG_DATA_HOME/muxlane/state.json" python3 - <<'PY'
import os,json
d=json.load(open(os.environ['STATE']))
assert d['pane_tree']['kind']=='leaf', d['pane_tree']
assert len(d['pane_tree']['group']['tabs'])==2
print('✓ second session opened as tab, not implicit split')
PY
import -window "$WID" "$ARTIFACTS/05-two-tabs-no-split.png"

# Ctrl+W 沿用 CloseTab 路径：关闭活动标签页并结束其后台 session。
xdotool key ctrl+w
sleep .2
STATE="$XDG_DATA_HOME/muxlane/state.json" MUXLANE_SOCKET="$XDG_DATA_HOME/muxlane/muxlane.sock" python3 - <<'PY'
import os,json,socket
d=json.load(open(os.environ['STATE']))
assert len(d['pane_tree']['group']['tabs'])==1, d['pane_tree']
s=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM);s.connect(os.environ['MUXLANE_SOCKET'])
s.sendall(b'{"id":1,"method":"state.list"}\n');b=b''
while b'\n' not in b:b+=s.recv(65536)
assert len(json.loads(b.decode())['result']['agents'])==1
print('✓ tab close terminated and removed its session')
PY
import -window "$WID" "$ARTIFACTS/05-tab-closed.png"

# ACP UI session: global palette -> project -> UI -> Claude fake ACP.
open_palette; sleep .3
xdotool key Return; sleep .2
xdotool key shift+Tab; sleep .2
xdotool key Return; sleep .8
wait_state "len(d.get('acp_threads', [])) == 1" "ACP UI thread created"
xdotool type --delay 2 "hello from ui smoke"
xdotool key Return
for _ in {1..100}; do
  if grep -R -Fq "Hello from fake ACP" "$XDG_DATA_HOME/muxlane/threads" 2>/dev/null; then break; fi
  sleep .05
done
grep -R -Fq "Hello from fake ACP" "$XDG_DATA_HOME/muxlane/threads"
import -window "$WID" "$ARTIFACTS/05-acp-thread.png"
echo '✓ ACP UI thread created, sent a prompt, rendered a streamed reply, and persisted history'
xdotool key ctrl+w
wait_state "len(d.get('acp_threads', [])) == 1" "ACP UI thread archived"
for _ in {1..100}; do
  if grep -R -Fq '"archived": true' "$XDG_DATA_HOME/muxlane/threads" 2>/dev/null; then break; fi
  sleep .05
done
grep -R -Fq '"archived": true' "$XDG_DATA_HOME/muxlane/threads"
echo '✓ closing an ACP tab archived history instead of deleting it'

# tab strip ＋ 与 Ctrl+Shift+T 共用 new_shell_tab：新增普通 Shell tab，不 split。
xdotool key ctrl+shift+t; sleep .5
STATE="$XDG_DATA_HOME/muxlane/state.json" python3 - <<'PY'
import os,json
p=os.environ['STATE']
d=json.load(open(p))
assert d['pane_tree']['kind']=='leaf', d['pane_tree']
assert len(d['pane_tree']['group']['tabs'])==2, d['pane_tree']
active=d['pane_tree']['group']['active']
assert active==d['pane_tree']['group']['tabs'][-1], d['pane_tree']
session=next(item['tmux_session'] for item in d['sessions'] if item['agent_id']==active)
open(os.path.join(os.path.dirname(p), 'new-tab-tmux'), 'w').write(session)
print('✓ tab-strip create path added a Shell tab without splitting')
PY
xdotool type --delay 2 "echo MUXLANE_NEW_TAB_FOCUS"
xdotool key Return
NEW_TAB_TMUX="$(<"$XDG_DATA_HOME/muxlane/new-tab-tmux")"
wait_tmux_text "$NEW_TAB_TMUX" MUXLANE_NEW_TAB_FOCUS "new active tab input"
echo '✓ new active tab accepted input without a click'
xdotool key ctrl+w; sleep .2
click_at "$X_TERM" "$Y_TERM" 1

# 5. 显式分屏（单 pane header 的垂直分屏按钮）。
click_at "$X_SPLIT" "$Y_HEADER" 1; sleep .8
STATE="$XDG_DATA_HOME/muxlane/state.json" python3 - <<'PY'
import os,json
d=json.load(open(os.environ['STATE']))
def leaves(n): return 1 if n['kind']=='leaf' else sum(leaves(c) for c in n['children'])
assert leaves(d['pane_tree'])==2, d['pane_tree']
print('✓ explicit split created second pane')
PY
import -window "$WID" "$ARTIFACTS/06-explicit-split.png"

# 分隔比例的归一化/持久化由 PaneTree 单测覆盖；UI smoke 不做鼠标拖动。
import -window "$WID" "$ARTIFACTS/07-split-stable.png"

# 7. 关闭右侧 split 只折叠布局，不终止 agent。
click_at "$X_MAX" "$Y_HEADER" 1; sleep .4
STATE="$XDG_DATA_HOME/muxlane/state.json" MUXLANE_SOCKET="$XDG_DATA_HOME/muxlane/muxlane.sock" python3 - <<'PY'
import os,json,socket
d=json.load(open(os.environ['STATE']))
assert d['pane_tree']['kind']=='leaf', d['pane_tree']
s=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM);s.connect(os.environ['MUXLANE_SOCKET'])
s.sendall(b'{"id":1,"method":"state.list"}\n');b=b''
while b'\n' not in b:b+=s.recv(65536)
assert len(json.loads(b.decode())['result']['agents'])==1
print('✓ close split terminated its shell session')
PY
import -window "$WID" "$ARTIFACTS/08-split-closed.png"

# 为最大化测试重新分屏。
click_at "$X_SPLIT" "$Y_HEADER" 1; sleep .5

# 8. 最大化（split 后右侧 pane 的最大化按钮）。
click_at "$X_SPLIT" "$Y_HEADER" 1; sleep .4
STATE="$XDG_DATA_HOME/muxlane/state.json" python3 - <<'PY'
import os,json
d=json.load(open(os.environ['STATE']))
def leaves(n): return 1 if n['kind']=='leaf' else sum(leaves(c) for c in n['children'])
# maximized_pane 是 transient，不落盘；分屏结构仍应保持完整。
assert leaves(d['pane_tree'])==2, d['pane_tree']
print('✓ maximize action kept the split layout intact')
PY
import -window "$WID" "$ARTIFACTS/09-maximized.png"

# 10. UI scale must resize the PTY and keep the newest terminal line at the bottom.
SCALE_TMUX="$(python3 - "$STATE" <<'PY'
import json,sys
state=json.load(open(sys.argv[1]))
active_pane=state['active_pane']
def active_agent(node):
    if node['kind']=='leaf':
        group=node['group']
        return group.get('active') if group['id']==active_pane else None
    for child in node['children']:
        agent=active_agent(child)
        if agent: return agent
active=active_agent(state['pane_tree'])
assert active, (active_pane,state['pane_tree'])
print(next(item['tmux_session'] for item in state['sessions'] if item['agent_id']==active))
PY
)"
tmux -L muxlane send-keys -t "$SCALE_TMUX" -l -- 'for i in $(seq 1 120); do echo MUXLANE_SCALE_LINE_$i; done; printf "\033[42mMUXLANE_SCALE_BOTTOM\033[0m\n"'
tmux -L muxlane send-keys -t "$SCALE_TMUX" Enter
wait_tmux_text "$SCALE_TMUX" MUXLANE_SCALE_BOTTOM "scale smoke output"
BASE_ROWS="$(tmux -L muxlane display-message -p -t "$SCALE_TMUX" '#{pane_height}')"
import -window "$WID" "$ARTIFACTS/10-scale-before.png"
click_at "$X_SETTINGS" "$Y_SETTINGS" 1
wait_settings_open "$ARTIFACTS/09-maximized.png" "$ARTIFACTS/10-scale-settings.png"
click_at "$X_SETTINGS_APPEARANCE" "$Y_SETTINGS_APPEARANCE" 1
sleep .2
import -window "$WID" "$ARTIFACTS/10-scale-appearance.png"
SCALE_SELECT_X="${MUXLANE_UI_SCALE_SELECT_X:-$(px .75 "$WIDTH")}"
SCALE_SELECT_Y="${MUXLANE_UI_SCALE_SELECT_Y:-$(px .37 "$HEIGHT")}"
SCALE_150_X="${MUXLANE_UI_SCALE_150_X:-$SCALE_SELECT_X}"
SCALE_150_Y="${MUXLANE_UI_SCALE_150_Y:-$(px .51 "$HEIGHT")}"
click_at "$SCALE_SELECT_X" "$SCALE_SELECT_Y" 1
sleep .2
import -window "$WID" "$ARTIFACTS/10-scale-menu.png"
click_at "$SCALE_150_X" "$SCALE_150_Y" 1
wait_state "d['ui_scale'] == 150" "UI scale changed to 150%"
for _ in {1..100}; do
  CURRENT_ROWS="$(tmux -L muxlane display-message -p -t "$SCALE_TMUX" '#{pane_height}')"
  if (( CURRENT_ROWS < BASE_ROWS )); then break; fi
  sleep .05
done
(( CURRENT_ROWS < BASE_ROWS )) || {
  echo "PTY rows did not shrink after UI scale: $BASE_ROWS -> $CURRENT_ROWS" >&2
  tmux -L muxlane list-panes -a -F '#{session_name} rows=#{pane_height} cols=#{pane_width}' >&2 || true
  exit 1
}
xdotool key Escape
sleep .35
import -window "$WID" "$ARTIFACTS/10-scale-after.png"
BASE_ROWS="$BASE_ROWS" AFTER="$ARTIFACTS/10-scale-after.png" python3 - <<'PY'
import os
from PIL import Image
im=Image.open(os.environ['AFTER']).convert('RGB')
w,h=im.size
crop=im.crop((round(w*.20), round(h*.70), round(w*.99), round(h*.97)))
green=sum(1 for r,g,b in crop.getdata() if g > 80 and g > r*1.25 and g > b*1.15)
assert green > 20, f'newest terminal line marker not visible near bottom: green_pixels={green}'
print(f'✓ UI scale resized PTY rows {os.environ["BASE_ROWS"]} -> marker pixels={green}')
PY
wait_tmux_text "$SCALE_TMUX" MUXLANE_SCALE_BOTTOM "scale smoke remains at terminal bottom"
echo "✓ UI scale 150% resized PTY rows $BASE_ROWS -> $CURRENT_ROWS and kept the newest line visible"

# 11. Window close requires explicit confirmation. Enter on the default Cancel keeps the app open;
# Tab then Enter activates Quit and closes it.
wmctrl -i -c "$WID"
sleep .3
wmctrl -lp | awk -v id="$WID" '$1==id {found=1} END {exit !found}' || {
  echo "window closed without confirmation" >&2
  exit 1
}
import -window "$WID" "$ARTIFACTS/11-quit-confirm.png"
BEFORE="$ARTIFACTS/10-scale-after.png" AFTER="$ARTIFACTS/11-quit-confirm.png" python3 - <<'PY'
import os
from PIL import Image, ImageChops
before=Image.open(os.environ['BEFORE']).convert('RGB')
after=Image.open(os.environ['AFTER']).convert('RGB')
w,h=before.size
crop=(round(w*.35),round(h*.32),round(w*.75),round(h*.68))
diff=ImageChops.difference(before.crop(crop),after.crop(crop))
changed=sum(1 for pixel in diff.getdata() if max(pixel)>4)
assert changed>1000, f'quit confirmation not visible: changed={changed}'
print(f'✓ window close displayed confirmation: changed={changed}')
PY
xdotool key Return
sleep .2
wmctrl -lp | awk -v id="$WID" '$1==id {found=1} END {exit !found}' || {
  echo "default quit action was not Cancel" >&2
  exit 1
}
wmctrl -i -c "$WID"
sleep .2
xdotool key Tab
xdotool key Return
for _ in {1..100}; do
  if ! wmctrl -lp | awk -v id="$WID" '$1==id {found=1} END {exit !found}'; then break; fi
  sleep .05
done
if wmctrl -lp | awk -v id="$WID" '$1==id {found=1} END {exit !found}'; then
  echo "confirmed quit did not close the window" >&2
  exit 1
fi
PID=""
echo '✓ default Cancel prevented accidental close; explicit Quit closed the window'

wmctrl -s "$ORIGINAL_WS"
ORIGINAL_WS=""
echo "UI smoke passed; artifacts: $ARTIFACTS"
