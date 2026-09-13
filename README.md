# Fluxter (Fluxer Terminal)

TUI for [Fluxer](https://fluxer.app), built with Ratatui.

## What this client does differently

fluxter began as a fork of
[dogbonewish/fluxer-tui](https://github.com/dogbonewish/fluxer-tui) and is
developed on its own now; that client is called "upstream" below. This one
keeps its config file, keys and slash commands and differs as follows, set
against upstream's master of April 2026. Each item has its own section or
table row further down.

Bugs and feature requests belong in [this client's own issue tracker](https://github.com/AIVirtuoso/fluxter/issues); upstream cannot
act on them.

**Login and packaging**

- **Browser login works again.** The handoff poll sends the
  `poll_secret` that the Fluxer API introduced in September 2026;
  without it the poll stays "pending" forever and trips the per-IP
  lockout within seconds.
- **A Nix flake** with the package, a dev shell and a `dev` app that
  tries a branch without recompiling every dependency; `cargo install
  --git` names the package (see "Install with cargo" and "Trying a
  branch without a full rebuild").

**Look**

- **The terminal's own colours by default.** The client uses the
  terminal's foreground, background and 16 ANSI colours, so it follows
  the terminal's theme, light or dark. `[ui] theme = "fluxer"`, or the
  Colours row in the settings overlay (**F2**), keeps upstream's fixed
  dark look.
- **Profile pictures beside messages, and pictures, GIFs and video
  posters under them**, like the web app, on sixel, kitty and iTerm2
  terminals and on the console. Previews are scaled by the media proxy
  before download and cached in memory and on disk within a budget; a
  picture cut by the pane's edge shows the part that is on screen (see
  "Pictures in chat"). Upstream shows a picture only on **Ctrl+O**.
- **Emoji shortcodes the Fluxer way.** About 900 of Fluxer's emoji
  names (`:slight_smile:`, `:thumbsup:`, ...) are unknown to the
  `emojis` crate and used to stay as raw text; they now render, complete
  in place while typing, and go out as the emoji. The `:` popup lists
  an exact name first, then prefixes, then substrings.
- **Custom community emoji as pictures.** `<:name:id>` in messages,
  reactions, the compose box and the `:` popup draws the actual emoji
  in terminals with graphics; animated ones play, every instance in
  step. Terminals without graphics keep the `:name:` text.
- **GIFs from the picker (KLIPY, Tenor) play in the terminal.** Upstream
  handed the provider's web page to an external player, so nothing
  played. Animated WebP is decoded as well as GIF, and the auth token
  is sent to the API host only, never to third-party media hosts.

**Reading**

- **The view stays still while you read history.** New messages and
  loaded history no longer push what you are reading up the pane; **G**
  jumps to the newest message.
- **Profile view.** **p** on a selected message shows its author's
  profile: badges, pronouns, bio, when they joined Fluxer and the
  community, roles, connections, mutual communities and friends. **p**
  again shows the profile picture full size.
- **Cheap scrolling.** The terminal shifts rows itself, only the rows
  that changed are sent, each frame goes out as one synchronized
  update, and a held key scrolls as fast as the terminal keeps up.
- **Messages drawn with Fluxer's own markup rules:** fenced code with a
  language, lists, tables, the five callouts, quotes, headings, small
  print, every inline mark in both spellings, backslash escapes, bare
  links, masked links that show where they go, timestamps in all nine
  styles (see "Message formatting"). Upstream's renderer knew a handful
  of inline marks and drew the rest as typed.
- **Mentions and replies highlighted** the way the web app does it: an
  amber bar in the margin (and a tint in the Fluxer theme) on messages
  that mention you or answer you, and the message a selected reply
  answers is marked (see "Interface overview").
- **A typing indicator that moves nothing:** who is typing is written
  into the compose box's bottom edge, costing no row, and stays there
  while you write. Upstream put the line inside the box, only while it
  was empty, and the whole screen shifted by a row every time someone
  started or stopped (see "Interface overview").
- **A pings list:** **p** opens the messages that mentioned you, newest
  first, across every community and direct message, the way the web
  client's inbox does; **Enter** jumps to one (see "Interface overview").
- **A "new messages" line**, and **U** to jump to it — so opening a busy
  channel no longer means scrolling and guessing where you left off.
- **Getting about by key.** **Alt+1**–**Alt+9** for a place in the left
  column, **Alt+Up**/**Alt+Down** to step through it from any focus,
  **Alt+Left**/**Alt+Right** back and forward through the channels you
  have visited, **Alt+L** between the last community and your
  conversations (see "Getting about").
- **The keys are always on screen.** The bar says what works where the
  focus is, every overlay says it on its own bottom line, and when
  something happens the words take that line for a few seconds before the
  keys come back — so sending a friend request tells you it was sent
  instead of saying it behind the list you are looking at.
- **Friends, requests and blocking.** **Alt+F** opens the four groups the
  server sorts your relationships into; **+** asks somebody by their tag,
  **a** accepts, **B** blocks, and a blocked account's messages stop
  appearing at all. The whole `/users/@me/relationships` surface was
  unused before, so there was no way to block anybody from here (see
  "Friends and blocking").
- **Who is online.** The client used to name `PRESENCE_UPDATE` only to
  keep it out of the debug log; it now follows presence and shows it, in
  the conversation list, beside the name on a message, in the profile and
  on the status bar for your own. `/status` sets yours and
  `/customstatus` the line under your name (see "Presence").
- **A member list.** **Alt+M** opens the roster of the open channel in a
  column beside the messages, grouped by hoisted role and then by who is
  about, the way the web client's member sidebar is. Members, joins,
  leaves and nickname changes arrive live; upstream fetched members only
  to complete an `@` and never showed them (see "The member list").

- **Everything the web client's right-click offers, on a key.** **a** on a
  selected message opens a menu of every action that message allows, and
  each has a direct key of its own: pin to the channel and read the
  channel's pins (**P**, **Alt+P**), bookmark and read your bookmarks
  (**b**, **Alt+B**), see who reacted and clear reactions (**v**), mark
  unread from a message, mark the channel or the whole community read,
  hide or show a message's link previews, copy a link to it or its id
  (**Y**), take one file off it, delete several at once (**m** marks
  them), and report it to the moderators. Upstream has none of these; the
  web client reaches them all through a pointer (see "Message actions").

- **Message search.** **/** searches this channel, this community or
  everywhere, with `from:`, `has:` and `pinned:` filters and quoted
  phrases typed straight into the query; **Enter** on a hit jumps to it in
  the chat. `POST /search/messages` was never called before, so there was
  no search of any kind (see "Searching").

- **Starting a conversation.** **Alt+N** opens one with anybody the client
  knows, or makes a group of several; **Alt+G** renames a group, adds
  somebody, takes somebody out or leaves it. Before this the client could
  read every conversation it already had and open none, because
  `POST /users/@me/channels` was never called (see "Conversations").

- **Invites, and joining a community.** **Alt+C** takes an invite, makes a
  community, browses the directory, lists a community's invites or leaves
  it. The client had no invite handling of any kind, so whatever you were
  already in was what you got (see "Communities and invites").

- **Voice, as far as a terminal should take it.** **Alt+V** joins a voice
  channel, rings a conversation, answers or turns down a call, mutes,
  deafens and leaves; an incoming call rings in the sidebar and raises a
  notification. The audio itself is handed to a program on PATH rather
  than carried in the client. Before this the client showed who was in a
  voice channel and could do nothing else, and threw every call event
  away unread (see "Voice").
- **Copy a message.** **y**, or **Ctrl+C**, on a selected message puts
  its text and the link of each file on it on the system clipboard
  through `wl-copy` or `xclip`, and always in the client's own cut
  buffer, so **Alt+V** pastes it into the compose box - the way on the
  Linux console, where there is no clipboard. Upstream has no way to
  copy a message, and **Ctrl+C** there only quits.
- **Others see you typing:** the client tells the channel while you
  write, with the web client's timing, and a switch (**F2** or
  `send_typing = false`) keeps it to itself. Upstream never sent it.
- **A performance mode that means it:** no pictures, no animations,
  slower ticks and one frame per burst of gateway traffic, for slow
  machines (see "Tips"). Upstream's skipped little.
- **Fixes:** the newest message is no longer cut off at the bottom of
  the pane; a sixel profile picture cut by the pane's edge leaves no
  stale strip behind; a community's notification settings save again
  (the API's answer to a change could not be read, so every change
  looked failed, and a saved entry emptied the client at the next
  start); and a community whose member list times out or is off limits
  no longer trips the client up, which keeps what arrived and says so;
  no longer trips the client up, which keeps what arrived and says so;
  a channel that received a message over the gateway before it was
  opened now loads its history (the client took the one message for the
  loaded history and showed nothing else); and a gateway connection that
  dies without saying so (a sleep, a network change) is noticed within
  15 s of an unanswered heartbeat and resumed, where before the client
  sat "Connected" with no messages, typing or presence arriving until
  the socket itself gave up.

**Sending**

- **A real editor in the compose box.** A cursor that moves by
  character, word and line, several lines with **Shift+Enter** or
  **Alt+Enter**, a selection (Shift+arrows, or **Ctrl+Space** and then
  the arrows, which also works on the console), cut, copy and paste,
  **Ctrl+B**/**Ctrl+I**/**Ctrl+S** and friends to put markdown around
  the selection or the word at the cursor, and undo/redo with
  **Ctrl+Z**/**Ctrl+Y**; the box scrolls to keep the cursor in view.
  Upstream only appends and deletes at the end (see "Editing the
  message you are writing"). Keyboard only: this client has no mouse
  support and will not get any.
- **Stickers, sent and shown.** **Alt+S** (or `/sticker`) opens a picker
  of every community sticker the client knows, filtered by name or tag,
  with the sticker under the cursor drawn beside the list; **Enter**
  puts it on the message and **Enter** in the input sends it, up to
  three per message and together with text and files. A sticker on a
  message someone else sent is drawn in the chat like any other picture
  (see "Stickers"). Upstream neither sends nor shows them.
- **Attach any kind of file.** A file picker (**Ctrl+F** or `/attach`),
  `/attach <path>`, and **Ctrl+V** for the image or the files on the
  clipboard (text on it is pasted into the message instead); up to ten per message, with previews of staged pictures
  and videos (see "Attaching files"). Upstream sends text only.

**Sound and notifications**

- **Audio attachments play** with **Ctrl+O** through an external player
  (mpv, ffplay, pw-play, paplay, aplay, or one you name), the same way
  in a terminal emulator and on the console (see "Audio").
- **Notifications** for mentions and direct messages through libnotify's
  `notify-send`, or GNU `mail` for the console when you opt in, with a
  **sound** through the audio player; nothing is announced for the
  channel you are reading while the window (or the VT) is in front (see
  "Notifications").

**Debugging**

- **A debug panel and a debug log.** `/debug` (or **F12**) shows the
  facts a bug report needs and the last log lines; `--debug` keeps the
  whole log in a file. Both record shapes, ids, sizes, timings and
  errors, never message text, names, paths or the token (see
  "Debugging").
- **A map of the screen for layout bugs.** `/debug frame` (or **f** in
  the panel) puts the frame into the log as one character per cell:
  border, picture, emoji, text or nothing, with the rows the compose
  box got against the rows it asked for. Where the text is, not what
  it says.

**Console**

- **Console mode.** On a Linux virtual console (`TERM=linux`) the whole
  UI is drawn through DRM/KMS with real fonts, colour emoji, custom and
  animated emoji, pictures and GIFs; no terminal emulator is needed
  (see "Console mode").

## Requirements

- Rust toolchain
- A terminal with reasonable size (the layout expects multiple panes);
  see "Terminals" for which ones have been tried and what they need
- Network access for the API, gateway WebSocket, and browser login

## Build and run

```bash
cargo build --release
# binary: target/release/fluxter
cargo run --release
```

## Install with cargo

The client installs straight from git. Name the package: the
repository also holds `scripts/gen-emoji-aliases`, a small tool that
generates the emoji alias table, and cargo asks which one to install
otherwise.

```bash
cargo install --git https://github.com/AIVirtuoso/fluxter fluxer-tui
# a branch: cargo install --git https://github.com/AIVirtuoso/fluxter --branch <branch> fluxer-tui
```

This puts `fluxter` in `~/.cargo/bin`. The crate uses edition 2024, so
Rust 1.85 or newer is needed; the console-mode dependencies (drm, swash)
are pure Rust, so no C libraries have to be installed. A few tools are
looked up on PATH at runtime and are optional: `chafa` for text-art
pictures on a terminal without a graphics protocol, `wl-copy` or `xclip`
for pasting, and `fc-match` (fontconfig) only when running on a Linux
virtual console, where the UI is drawn through DRM (see `[console]`
below).

## Install with the PKGBUILD on Arch (or its derivatives)

This is good for those who use an Arch-based distro and want to manage their
install with Pacman. This requires `wget` to be installed.

```bash
cd $(mktemp -d)
wget https://raw.githubusercontent.com/AIVirtuoso/fluxter/refs/heads/master/PKGBUILD
makepkg -si
```

A few tools are looked up on PATH at runtime and are optional:
`chafa` for text-art pictures on a terminal without a graphics
protocol, `wl-copy` or `xclip` for pasting, and `fc-match`
(fontconfig) only when running on a Linux virtual console,
where the UI is drawn through DRM (see `[console]` below).

## Trying a branch without a full rebuild

`nix run github:AIVirtuoso/fluxter/<branch>` builds the package in the
Nix sandbox, where every new commit compiles all dependencies again. For
quick tests use the `dev` app instead:

```sh
nix run --refresh github:AIVirtuoso/fluxter/<branch>#dev
```

It copies the snapshot to `~/.cache/fluxer-tui/src-…` (cargo tells fresh
from stale by file dates, and store files have none) and builds it with
cargo in `~/.cache/fluxer-tui/target`, so only the crates that changed are
recompiled (the first run compiles everything once, in release mode). `--refresh` makes Nix look the branch up again instead of
reusing the commit it cached for an hour. From a checkout, `nix run .#dev`
does the same.

## First-time login

If you have no valid saved token then you can easily login via the browser. The TUI will automatically:

1. Print a **login code** (twelve characters in two groups, also copied to the clipboard when possible).
2. Opens your browser to complete login (or you can open the printed URL manually).
3. Polls until the browser flow finishes, then saves the token and starts the UI.

**"browser login is blocked from this IP address for now."** The
server counts failed handoff attempts per IP address and refuses every
handoff request for 15 minutes after the fifth failure, answering
`INVALID_HANDOFF_CODE` even for a code it has just issued. Failures
come from a code typed wrongly on the login page, a code from a
fluxter that had already exited, or an older client (upstream
0.7.5) polling without the poll secret. fluxter stops as soon as it
sees this instead of waiting five minutes for nothing. Wait 15 minutes
without trying again, since every wrong attempt starts the 15 minutes
over, then run it once more and enter the code exactly as shown.

You can pass a token once without storing it in config:

```bash
target/release/fluxter --token 'YOUR_TOKEN_HERE'
```

To clear the saved token and exit:

```bash
target/release/fluxter --logout
```

`--debug` keeps a debug log, `--debug-log FILE` says where, and
`--no-graphics-query` skips the terminal's picture-protocol query; see
"Debugging".

Or you can CTRL + L (see below)

## Command-line options

| Option                 | Description                                                              |
| ---------------------- | ------------------------------------------------------------------------ |
| `--token <TOKEN>`      | Use this token for this run (still written to config if login succeeds). |
| `--config <PATH>`      | Config file path (default: see below).                                   |
| `--api-base-url <URL>` | API base URL (default: `https://api.fluxer.app/v1`).                     |
| `--logout`             | Clear stored token from config and exit.                                 |

## Config file

Default path (unless you pass `--config`): **`…/fluxer-tui/config.toml`** under the OS config directory (on Linux this is usually **`~/.config/fluxer-tui/config.toml`**).

It stores `api_base_url`, `token`, `last_server_id`, and `last_channel_id` so the client can restore your last place, plus the `[ui]`, `[media]` and `[console]` settings described below.

`api_base_url` is always read as an `https://` address. A bare host gets the
scheme put in front of it (`api.fluxer.app/v1` becomes
`https://api.fluxer.app/v1`), and an `http://` one stops the client at start
with a message naming the key instead of being quietly upgraded to something
you did not write: the login token goes out in the headers of every request,
and a plaintext address would put it on the wire for anyone on the path to
read. `--api-base-url` on the command line is read the same way.

## Pictures in chat

On a terminal that can draw pictures (sixel, kitty, iTerm2) and on the
console, the message pane looks like the web app (see "Terminals" for
which ones have been tried):

- **Profile pictures** sit to the left of each message, two rows tall; a
  user without one gets the web app's default avatar. They are round on
  the console, kitty, iTerm2 and sixel; on halfblocks they are round when
  there is a colour to flatten their corners onto (see "Transparent
  pictures" below).
- **Pictures, GIFs and video posters** are shown under the message as
  previews: scaled to fit about a third of the pane, never larger than the
  original. GIFs from the picker (KLIPY, Tenor) and animated WebP play.
  **Ctrl+O** on a selected message still opens the full-size picture or
  GIF, videos externally, and plays audio (see "Audio"). Performance mode
  draws no pictures at all (see "Tips").

Previews are asked from Fluxer's media proxy already scaled to the size
they are drawn at, so a 4000-pixel photo costs a few kilobytes. Only the
pictures on screen are fetched (a few at a time); what is decoded stays in
memory within a budget and the least recently drawn go first; the
downloaded bytes are kept on disk under `~/.cache/fluxer-tui/media` within
a size cap, the oldest files evicted first. Nothing else is written.

```toml
[ui]
inline_media = true # pictures and GIFs under messages
avatars = true # profile pictures beside messages
image_background = "" # see "Transparent pictures" below

[media]
disk_cache_mb = 64 # 0 turns the disk cache off
memory_cache_mb = 64
audio_player = "" # see "Audio" below
```

Both `[ui]` switches are also in the settings overlay (**F2**). A picture
cut by the pane's edge shows the part that is on screen (on sixel, whole
six-pixel bands of it).

### Transparent pictures

Sixel and halfblocks carry no alpha channel, so anything transparent — the
corners of a round avatar, a sticker, a PNG with a hole in it — has to be
dealt with before the picture is drawn. Left alone, the encoder keeps
whatever colour sits under the alpha, which is black in most files.

On sixel the client simply does not draw those pixels. A position a sixel
never sets keeps what the terminal already had there, and the cells under a
picture are blanked first, so the transparency shows your real background
exactly. That is worth doing rather than painting the background colour on:
the encoder holds only five bits per channel, so a background of `#002b36`
would come out as `#002830`, close enough to read as a faint box around
every avatar.

Halfblocks have no such way out, so there the transparency is flattened
onto a colour instead: the Fluxer theme's background where the theme fixes
one, and the terminal's own otherwise, which the client asks for at start
with an OSC 11 query. foot, xterm, kitty, wezterm, Konsole and most others
answer it.

```toml
[ui]
# "" or "auto": leave transparency undrawn on sixel; elsewhere flatten it
#               onto the theme's background, or the terminal's own
# "none":       no flattening anywhere; the protocol keeps what is under
#               the alpha
# a colour:     flatten onto it and draw it — "#002b36", "002b36" or
#               "rgb:00/2b/36" — the way out for a sixel terminal that
#               paints unset positions instead of leaving them alone
image_background = ""
```

A terminal that answers nothing costs a quarter of a second at start and
says so in the status bar; the debug log records the answer either way. On
the console, kitty and iTerm2 the pictures keep their own alpha and the
setting changes nothing.

Scrolling is cheap on a terminal: when the pane merely scrolled, the
terminal is asked to shift those rows itself (pictures move with them, as
foot, kitty and xterm do), and only the rows that came into view are
sent. Keys that arrive while a frame is drawn are handled together, so a
held key scrolls as fast as the terminal keeps up.

## Terminals

The client runs on anything ratatui can drive. What changes between
terminals is how pictures are drawn and whether the Alt shortcuts arrive at
all. This table says how far each one has actually been tried, which is not
the same as how well it is expected to work: most of the list has never
been run.

| Terminal                                   | Pictures                                                       | How far it has been tried                                                                                                                                                                                                                                                                                                                                      |
| ------------------------------------------ | -------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **foot**                                   | sixel                                                          | Everything in this README has been used on it. The transparency was checked against the screen pixel by pixel.                                                                                                                                                                                                                                                 |
| **Linux console** (a tty, no X or Wayland) | the client draws them itself through DRM                       | Used regularly; see "Console mode" below.                                                                                                                                                                                                                                                                                                                      |
| **xterm**                                  | sixel, once told to be a VT340                                 | Used with pictures, stickers and animations, and checked against the screen. It leaves unset sixel positions alone, so transparency works; it cannot hold a frame back until it is whole, so a large picture can be caught part-drawn. It has no coloured underlines (SGR 58) either, which the client is careful never to rely on. Needs the resources below. |
| **tmux**                                   | whatever tmux itself manages; halfblocks when it does no sixel | Only used for the project's own headless tests, where it comes out as halfblocks.                                                                                                                                                                                                                                                                              |
| kitty, iTerm2                              | their own protocols, which carry transparency                  | Never tried. Nothing has to be flattened on these, so `[ui] image_background` does nothing at all.                                                                                                                                                                                                                                                             |
| WezTerm, Konsole, mlterm, Contour, mintty  | sixel                                                          | Never tried.                                                                                                                                                                                                                                                                                                                                                   |
| alacritty                                  | none of them; falls back to halfblocks                         | Never tried.                                                                                                                                                                                                                                                                                                                                                   |

If a sixel terminal paints the transparent parts of a picture instead of
leaving them alone, set `[ui] image_background` to your background colour;
see "Transparent pictures" above.

A terminal that cannot hold a frame back until it is whole — xterm is one,
foot and kitty are not — draws a picture as it arrives, so a large one can
be caught half drawn. Nothing is left of an earlier frame when that
happens: an animation's frames all paint the same area, so each replaces
the one before outright.

It also changes how the message pane is scrolled. Where a frame is shown
whole, the terminal is asked to shift the pane's rows itself, which keeps
the pictures in them on screen without sending them again; that shift moves
every column of those rows, so the Servers and Channels boxes go with it
and are written back in the same frame. Where a frame is not held back
that would be seen happening — both boxes flickering on every scroll step —
so the pane is redrawn instead and the boxes are left alone. The cost is
that a picture in view is sent again as it moves.

The shift reaches further than the rows it is given: a terminal redraws
every picture on the screen at the scrolled offset and cuts the result to
the pane, so a thumbnail staged in the compose box, or a picture on an
overlay, paints a slice of itself into the rows arriving at the pane's
edge. Those rows are written back over whatever landed on them, which is
why they go out even when they hold nothing at all. Without that, slices
of the picture stayed wherever no text was laid over them — between the
words of a message, and after the last one — and rode up the pane on
every further scroll.

### xterm

On the command line or in `~/.Xresources`; the first two matter, the
third only improves the colours:

```
XTerm*decTerminalID: vt340       # without this there is no sixel at all
XTerm*metaSendsEscape: true      # without this Alt+S arrives as "ó"
XTerm*numColorRegisters: 1024    # optional; 256 is the default
```

The second one is not about this client. xterm's `eightBitInput` defaults
to true, so a key pressed with Meta is delivered as one byte with its
eighth bit set: Alt+S becomes `0x73 | 0x80`, which is `0xf3`, the character
`ó`. Every Alt shortcut in the tables below is affected, not just the
sticker picker. The client cannot reasonably guess at this, since `ó` is a
character people type. The slash commands (`/sticker` and the rest) need no
modifier and work either way.

Two further xterm defaults are about keys rather than pictures.

**Alt+Enter never arrives.** xterm's own translations bind
`Alt <Key>Return` to its `fullscreen()` action, so the window toggles
full screen and the compose box hears nothing. **Ctrl+J** starts a new
line as well and xterm passes it through untouched, so there is nothing
to configure; to have the key itself back, override the translation:

```
XTerm*VT100.translations: #override Alt <Key>Return: string(0x1b) string(0x0d)
```

**Backspace sends `^H`, and Ctrl+Backspace sends DEL** - the other way
round from every other terminal here - because xterm's `backarrowKey`
defaults to true. The client reads `^H` (which reaches it as Ctrl+H) as a
Backspace, so both delete one character; the word before the cursor goes
on **Ctrl+W** or **Alt+Backspace**. `XTerm*backarrowKey: false` makes the
key send DEL like elsewhere. Outside the input Ctrl+H still opens the
keybindings overlay, so Backspace there opens it too.

## Voice

**Alt+V** opens the voice menu. What it offers depends on where you are
and what is going on:

| Row | When |
| --- | ---- |
| **Join this voice channel** | A community's voice channel is open |
| **Ring this conversation** | A one-to-one or a group is open |
| **Answer the call** / **Turn the call down** | Something is ringing for you |
| **Mute** / **Deafen** and their undos | You are in a call |
| **Leave** | You are in a call |
| **Copy the connection details** | You are in a call and the grant has arrived |

A conversation that is ringing says so beside its name in the list, and
raises a notification like a mention. Turning a call down stops it
ringing for you alone; it goes on ringing for everybody else.

### The client does not carry the sound

Fluxer's voice media is **LiveKit**. The gateway hands a session a
signalling URL and a token, and everything after that is WebRTC — ICE,
DTLS-SRTP, an Opus codec. This client does not speak any of it, on
purpose: bringing it in would mean libwebrtc, a C++ toolchain in the
build, and a great deal of code that has nothing to do with drawing a
terminal.

So voice follows the same division as the audio player and the
notification sender, which is the rule the rest of this client is built
on: **the client decides and a program on PATH does it.** fluxter joins,
leaves, mutes, deafens, answers, rings and keeps the bookkeeping, and
hands the URL and token to whatever `[media] voice_command` names.

```toml
[media]
voice_command = "livekit-cli join-room --url {url} --api-key '' --token {token} --publish-microphone"
```

Three placeholders are filled in: `{url}`, `{token}`, and `{key}` for the
end-to-end key where the channel has one. **The command is split into
arguments before the values go in**, so nothing the server sends can add
an argument of its own however it is punctuated.

**With no `voice_command` set you still join** — you appear in the
channel, others see you there, and you can mute and leave — but no sound
goes either way. That is a real state rather than a failure, so the menu
says `no sound is being carried` in red and names the setting, and the
status bar shows the call either way.

The grant is a credential. The debug log records its shape and never its
content, and **Copy the connection details** says so when it puts it on
the clipboard: it is there for setting a player up by hand, not for
pasting into a bug report.

## Audio

**Ctrl+O** on a message with an audio attachment (a voice message, an
mp3, ...) plays it; **Ctrl+O** on it again stops. The client does not
decode audio itself: the file is piped to a program that does, with no
terminal of its own, so it works the same in a terminal emulator and on a
Linux console. With nothing configured the first of these on PATH is
used, each told to play audio only and read stdin:

```
mpv --no-video --no-terminal --really-quiet -
ffplay -nodisp -autoexit -loglevel error -
pw-play -
paplay
aplay -q
```

Any other program that reads the audio from stdin does too:

```toml
[media]
audio_player = "sox -q -t mp3 - -d"
```

The status line shows what plays. Audio attachments are listed with ♪
and their length.

## Attaching files

Any kind of file can go with a message, up to ten at a time:

- **Ctrl+F**, or `/attach` on its own, opens a file picker: directories
  first, type to filter (a leading dot shows dotfiles), **Enter** opens a
  directory or attaches the file, **←** or **Backspace** on an empty
  filter goes up. What is under the cursor is described on the right,
  with a preview when it is a picture or a video.
- `/attach ~/path/to/file` attaches by path.
- **Ctrl+V** attaches what is on the clipboard: an image, or the files
  copied in a file manager (`wl-paste` or `xclip` do the reading).

Staged files show above the text you type, pictures and videos as
thumbnails where the terminal draws pictures, everything else by name and
size. **Ctrl+O** shows the last staged picture or video full size,
**Ctrl+X** drops it (when nothing is selected in the text, and when no
sticker is staged), **Enter** sends text and files together. A video's
preview is its first frame, which `ffmpeg` on PATH provides; without it
videos are listed by name.

## Stickers

A sticker is a picture a community stores for its members to send. The
client reads them from the gateway as the communities arrive, so the
picker is ready without asking the API for anything.

- **Alt+S**, or `/sticker` on its own, opens the picker: the active
  community's stickers first, then the ones of every other community the
  client knows, each under its name. **j**/**k** (or **↑**/**↓**) move,
  **g**/**G** jump to the first and the last, **Ctrl+D**/**Ctrl+U** by
  ten; **Enter** (or **l**) puts the sticker on the message; **q**,
  **h** or **Esc** closes. The sticker under the cursor is drawn on the
  right where the terminal draws pictures.
- **/** searches by name or tag: the list narrows as you type, **Enter**
  keeps the search and goes back to moving with the vim keys, **Esc**
  leaves the search and puts the list back as it was, **Backspace**
  edits it (**Ctrl+U** clears it).
- `/sticker <name>` opens the picker with the search already filled in.
- A message carries **at most three** stickers, along with text and
  files. **Enter** in the input sends everything together; **Ctrl+X**
  drops the last staged sticker (and, when none is staged, the last
  staged file).
- Stickers on messages are drawn in the chat under their name, at the
  size a picture preview gets, animated ones playing. Where the
  terminal draws no pictures, or with `inline_media = false`, the name
  is shown alone. A sticker the server classifies as explicit says so
  beside its name.

Sending a sticker of another community, or any custom sticker in a direct
message, is a Fluxer Premium feature: the server refuses it and the
client shows what it said. Communities you are in can always use their
own.

## The member list

**Alt+M** opens the roster of the open channel in a column to the right
of the messages, and closes it again. **Alt+J** and **Alt+K** scroll it.
The column has no focus of its own — it is a thing to read beside the
messages, not a place the **Tab** cycle stops at.

It is grouped the way the web client's member sidebar is: each hoisted
role in role order with its count, then everybody else who is about under
**Online**, then **Offline**. People who are away are dimmed. The box's
title counts the two: `12 of 40`.

The list follows you from channel to channel, because who can see a
channel decides who is in its list. In a direct message it closes itself,
since there is no list to show; on a terminal narrower than 80 columns it
is not drawn at all rather than squeezing the messages.

### How it arrives

The gateway does not send a whole roster. A client asks for windows of it
with a Lazy Request (op 14) — this one asks for rows 0 to 99, which is
the most the server allows in one window and more than a terminal shows —
and the server answers with `GUILD_MEMBER_LIST_UPDATE`, whose `SYNC`
operations each replace one inclusive range of rows. So the list is kept
as a sparse map by position rather than a list of people, and a row no
window has covered is simply not known yet.

Two consequences worth knowing:

- **A community's `Offline` group is left out once it holds more than a
  thousand people**, and its members are left out of the rows with it. The
  count in the title still counts them, so `12 of 4000` with far fewer
  than 4000 rows to scroll is the server being terse, not a bug.
- **The list needs `VIEW_CHANNEL` and `VIEW_CHANNEL_MEMBERS` on the
  channel.** Without both the server sends nothing, and the column says
  it is still loading.

The presences the list carries are folded into the client's own, so
opening it on a channel also fills in the dots on that channel's
messages.

## Friends and blocking

**Alt+F** opens the list. It has the four groups the server sorts
relationships into, and **←** / **→** move between them:

| Group | Who is in it | Keys |
| ----- | ------------ | ---- |
| **Friends** | People you are friends with | **Enter** opens the conversation, **n** gives them a name of your own, **x** unfriends, **B** blocks |
| **Wanting** | People who have asked you | **a** accepts, **x** turns them down, **B** blocks |
| **Asked** | People you have asked | **x** takes the request back |
| **Blocked** | People you have blocked | **x** unblocks |

**+** in any group asks somebody by their **tag** — `name#0001`, shown
beside every name in the list — which is how you reach an account you
share nothing with. **R** reloads, **Esc** closes.

The same three keys are on a profile (**u** on a message): **+** asks
them to be friends, **B** blocks them, and **x** undoes whichever tie
there is. A profile also says how you stand with them.

### What blocking does here

**A blocked account's messages are not kept at all.** They are dropped as
they arrive over the gateway and filtered out of every page fetched from
the API, which is what keeps them out of the pane, out of the
notifications and out of the unread counts without a check in any of
those places. Blocking somebody also takes what they have already said
out of the loaded channels, since otherwise the old messages would sit
there while new ones vanished.

The consequence is that **unblocking cannot bring back what was thrown
away**: the channels they were in are marked unloaded and fetched again
from the server. That happens on its own; there is nothing to press.

A name you give a friend with **n** is stored on the server, so it
follows you to the web client. An empty one drops it again.

## Presence

Fluxer tells a client who is about through `PRESENCE_UPDATE`, and through
the presences that come with the session at start. This client follows
them and shows them in four places:

- **The conversation list.** A one-to-one conversation shows the other
  person's state in place of its `@`.
- **Beside a name on a message**, on the row that names the author.
- **In the profile** (**u** on a message), where the state is written out
  in words, along with "on a phone" when their presence came from one and
  the line they have set under their name.
- **On the status bar**, for your own.

**Two glyphs carry it, not four**, with colour separating the rest:

| | State | Colour |
| - | ----- | ------ |
| `●` | online | green |
| `●` | idle | amber |
| `●` | do not disturb | red |
| `○` | offline or invisible | dim |

That is deliberate. The shape carries the distinction that matters at a
glance — about, or not — and is the one a terminal with no colour can
still read; the two characters are in every monospace font worth the
name, where a third and fourth rarer glyph come out as missing-character
boxes on some of them. Anywhere the state is worth spelling out it is
also written in words, so colour is never the only thing saying it.

**Nothing is drawn for somebody the server has said nothing about.** On a
message, only a person who is actually about gets a mark: a hollow circle
on every other message would be noise, and would claim somebody is away
when the truth is that the client has not been told either way. The
server only sends presences for people you share a community or a
conversation with.

### Setting your own

```
/status online        /status idle        /status dnd        /status invisible
/customstatus writing it up
/customstatus                             (clears the line)
```

`/status` writes to your account, so it follows you to the web client and
back. **Invisible** is how you appear offline while still receiving
everything; there is no `/status offline`, because that is not a thing
you choose. `/customstatus` takes up to 128 characters; setting an emoji
on it is not wired up here, but one set elsewhere is shown.

## Getting about

The left column has the conversation list first and then the communities,
and these keys reach it without moving the focus there:

| Key | What it does |
| --- | ------------ |
| **Alt+1** … **Alt+9** | The first nine places in that column. **Alt+1** is always the conversation list, **Alt+2** the first community, and so on. The numbering is the web client's. |
| **Alt+↑** / **Alt+↓** | Previous / next entry in the column, wrapping at both ends. |
| **Alt+←** / **Alt+→** | **Back** and **forward** through the channels you have visited, the way a browser's buttons work: fifty deep, and going somewhere new from the middle throws away what was ahead. The bar offers whichever way is open, and nothing when neither is. |
| **Alt+L** | Between the community you were **last** in and the conversation list. |

A channel that has gone since you visited it is not moved to; the client
says so and the entry stays, in case it comes back.

## The new messages line

An amber rule across the pane, with **new messages** in the middle, above
the first message that arrived since you last opened the channel.
**U** jumps to it from anywhere in the client.

Three things about where it sits, because they are the ones that make it
useful rather than annoying:

- **It stays where it was while you read.** The client marks messages read
  as you look at them, so a line that followed the read state would slide
  away under you. It is fixed when you open the channel and does not move
  until you leave.
- **It goes once you have caught up** — at the bottom of the channel with
  nothing unread. It is not there next time unless something new arrived.
- **A channel you have never opened gets none.** A rule above the whole
  history says nothing, and would cost a row saying it.

## Knowing what to press

Two places say what the keys do, and neither needs **F1**:

- **The bar under the title**, for wherever the focus is — a different
  line for the servers, the channels, the messages with and without a
  selection, and the compose box with and without a selection.
- **The bottom line of every overlay**, for the keys that work in it:
  the profile, the friends list, the pins, the bookmarks, the pickers,
  the settings, the debug panel and the keybindings overlay included.
- **The title of the `:`, `@` and `/` popups**, which sit straight on top
  of the compose box and have no row to spare — the keys go there
  instead, in the longest wording that fits the popup's width.
- **The compose box's own title**, which says how to finish and how to
  leave whichever mode it is in: replying, editing or forwarding. It
  named the mode before this and left you to guess that **Esc** gets out.
- **The bar**, for the member column, which has no focus of its own and
  so nowhere else to advertise **Alt+J** / **Alt+K**.

**A line only offers what would do something.** On a profile the three
relationship keys change with how you stand with the person: a stranger
gets `+ add friend` and `B block`; a friend gets `x unfriend` instead of
`+`; somebody who asked you first gets `+ accept` and `x turn down`; a
request you sent gets `x take it back`; somebody blocked gets `x unblock`
and nothing else. Your own profile gets none of the three.

That list is not written out twice. The keys and the line read the same
function, so the line cannot come to offer something the key would not
do — which is how `+` came to send a fresh friend request at somebody who
had already asked, where the server wants an accept.

When something happens — a friend request sent, a message pinned, a
community joined — the words take that line for four seconds and then the
keys come back. That matters because **an overlay covers the status bar**:
before this, sending a friend request from the friends list said so behind
the list you were looking at, which is to say it said nothing. An error is
the exception and stays until something replaces it.

## Message actions

The web client puts a message's actions behind a right-click. This client
is keyboard-only, so **a** on a selected message opens the same list and
walks it with **↑** / **↓**; **Enter** chooses, **Esc** steps back out of
a list the menu led to and closes it from the top. Only the rows that
apply are offered, so somebody else's message shows no Edit and a channel
without **Manage Messages** shows no Clear reactions.

The frequent ones also have a key of their own, shown on the right of each
row, and those work straight from the message pane without the menu.

| Action | Key | What it does |
| ------ | --- | ------------ |
| Add a reaction | **e** | The emoji picker, aimed at that message. |
| Who reacted | **v** | The names behind one reaction. **←** / **→** walk the message's other reactions, **x** clears everybody's reaction with the one shown (needs **Manage Messages**). |
| Clear every reaction | | Takes them all off. Asks a second time first, and needs **Manage Messages**. |
| Reply, Forward, Edit | **r**, **f**, **Ctrl+E** | As before. |
| Pin / Unpin | **P** | Pins the message to the channel. Yours anywhere you can post, anybody's with **Manage Messages**. |
| Pinned messages | **Alt+P** | The channel's pins, newest first; **Enter** jumps to one, **x** unpins. Opening it marks the channel's pins seen, so the channel stops counting as having new ones. |
| Bookmark | **b** | Saves the message to a private list of your own. Nobody else sees it, and it is not the same thing as a pin. |
| Bookmarked messages | **Alt+B** | Your bookmarks from everywhere. An entry whose message has since been deleted says so rather than disappearing, so you can still drop it with **x**. |
| Mark unread from here | | Acknowledges the channel up to the message *above* the one picked, so that one is the first thing still unread. |
| Mark the channel read | | Acknowledges up to the newest message the client has. |
| Mark the community read | | The same for every unread channel of the community at once. |
| Hide / show the link previews | | Sets or clears the message's `SUPPRESS_EMBEDS` flag. Your own messages only. |
| Copy the text | **y** | As before. |
| Copy a link to it | **Y** | The web app address of the message, the thing you paste to point somebody at it. |
| Copy the message id | | The raw snowflake. |
| Remove a file from it | | Takes one attachment off without deleting the message. With several files it asks which. Your own messages only. |
| Delete | **Ctrl+D** | As before. |
| Delete the marked messages | | **m** marks a message (a red cross appears in its margin); this deletes every marked message of the channel in one call. Asks a second time first. Needs **Manage Messages** and at least two marks, and the server refuses messages more than two weeks old. |
| Report to the moderators | | Asks which of the server's twelve categories, then sends it. Other people's messages only. |

Pins, bookmarks, bulk deletes and cleared reactions arrive over the
gateway as well, so a change made in another client shows here without a
reload.

## Searching

**/** opens the search. Type the query, pick the scope with **←** and
**→**, and press **Enter**. The cursor then moves down among the hits:
**↑** and **↓** walk them, **Enter** jumps to one in the chat, **n** and
**p** turn the page, **/** puts the cursor back in the query without
losing the results, and **Esc** closes.

Three scopes, and it opens on the narrowest one that makes sense from
where you are:

| Scope | What it covers |
| ----- | -------------- |
| **this channel** | The channel you are reading |
| **this community** | Every channel of it you can see |
| **everywhere** | Every community you are in and every conversation you have had |

### What a query can say

The web client builds its filters by clicking pills. With a keyboard the
same filters are worth typing, so they go in the query the way a mail
client or a code host does:

```
sixel transparency                 words
"no more combinations"             has to appear together
from:ada                           only their messages
from:ada#0001                      by tag, when two people share a name
has:image                          also sound, video, file, embed
pinned:true                        pinned messages only
has:file from:ada "the patch"      all of it at once
```

`from:` matches a nickname in the community you are in first, then a
display name, then a username — and says so rather than searching for the
wrong thing when it matches nobody. An unknown `has:` value, like
`has:sausages`, is not a filter at all and is searched for as text, which
is friendlier than refusing the query.

### Two things the server does

**It may answer "still indexing".** A channel Fluxer has not finished
indexing makes the whole search come back as `{"indexing": true}` rather
than results. That is an answer, not a failure: the overlay says so and
**Enter** tries again.

**A hit can be somewhere you have not opened.** The answer carries the
channels its hits are in, so a result from a community you have never
scrolled still says where it came from. Jumping to one in a community you
have since left says so instead of moving the pane somewhere wrong.

## Conversations

**Alt+N** starts one. It lists everybody the client knows — the people you
are already talking to, and the members of every community you are in —
and typing narrows the list by display name, username or tag. **Enter**
opens the conversation with the person under the cursor; the server hands
back the one you already have rather than a second, so this is also the
way back to a conversation you closed.

**Space** ticks somebody instead. Tick two or more and **Enter** makes a
**group** of them and you.

### Looking after a group

**Alt+G** on an open group offers what a group needs: **rename it** (an
empty name drops the name and it goes back to listing its people), **add
somebody** (the same people list, but Enter puts them in the group),
**take somebody out**, and **leave it**.

### In the conversation list

| Key | What it does |
| --- | ------------ |
| **P** | Keep the conversation at the top of the list, or let it go. Pinned ones sort above the rest and carry a small `·pin`; within each half the newest message is still first. |
| **x** | Close the conversation, or leave the group. |

**Closing deletes nothing.** The messages stay on the server and the
conversation comes back the moment either side writes; **Alt+N** reopens
it. Leaving a group is the same call and does take you out of it.

People joining and leaving a group arrive over the gateway, so a change
made in another client shows here without a reload.

## Communities and invites

**Alt+C** opens the menu. What it offers depends on where you are: the
first three are always there, the last two only inside a community.

| Row | What it does |
| --- | ------------ |
| **Join with an invite** | Takes a bare code or a pasted link — from this instance or any other, with or without a scheme, and with anything after the code (`?utm=…`, `#top`) ignored. |
| **Make a community** | Asks for a name and makes it. You are its owner. |
| **Browse the directory** | The instance's discovery listing. **/** searches it, **Enter** joins straight from the list without an invite. |
| **Invites to this community** | Every invite you may see. **y** copies its link, **+** makes one to the channel now open, **x** revokes. |
| **Webhooks in this community** | Every webhook and the channel it posts into. **+** makes one in the channel now open, **r** renames, **y** copies its address, **x** deletes after asking. Needs **Manage Webhooks**. |
| **Leave this community** | Leaves it. A community you own cannot be left, and the client says so rather than sending a call the server would refuse. |

### An invite is looked up before it is taken

Typing a code does not join anything. The client fetches the invite first
and shows what it leads to — the name, the description, how many are
online of how many members, who made it, when it runs out, and whether
the membership is temporary — and **Enter** on that takes it. That is one
extra keystroke and it means nobody joins something whose name they have
not seen.

An invite made with **+** is a day long with no limit on uses, which is
what the server itself defaults to, and its link goes on the clipboard as
soon as it exists.

### A webhook's address is a secret

**y** copies it; nothing shows it. A webhook's token is minted once and
never rotated, so the address -- `.../webhooks/<id>/<token>` -- is a bearer
credential: anything holding it can post into that channel as that webhook,
with no account and no permission check. It is not drawn on the screen, and
it never reaches the debug log either. A test asserts that no row of the
list contains it.

Deleting a webhook is the only way to revoke one, which is why **x** asks
first, with the cursor on "No".

## Message formatting

Messages are drawn with the markup Fluxer's own parser understands, so
what you see is what the web app shows:

- **Inline:** `**bold**`, `*italic*` or `_italic_`, `***both***`,
  `__underline__`, `~~strikethrough~~`, `||spoiler||` (dimmed), `` `code` ``
  (double backticks keep a backtick inside), and `\` before any marker
  to show it as typed. An underscore inside a word is just an underscore.
- **Links:** bare `https://` addresses, `<https://...>`, and
  `[label](https://...)`, which shows the label underlined with the
  link's host in brackets, so a label cannot pretend to be somewhere
  else; a label that is itself a different address is shown as written.
- **Mentions and more:** `@user`, `@role`, `#channel`, `@everyone`,
  `@here`, slash commands, `<t:...>` timestamps in all nine styles in
  your local time (and `R` as "3 hours ago"), community emoji and
  `:shortcodes:`.
- **Blocks:** `#`, `##`, `###`, `####` headings; `-#` small print;
  `>` quotes and `>>>` for the rest of the message; `> [!NOTE]`,
  `[!TIP]`, `[!IMPORTANT]`, `[!WARNING]` and `[!CAUTION]` callouts with
  a badge and a coloured bar; `-`, `*` and `1.` lists, nested by two
  spaces of indent; fenced code blocks with an optional language, drawn
  with a bar in the margin; `||` spoilers spanning several lines; and
  tables with `|` cells and `:--`, `:-:`, `--:` alignment.

Bold, italic and the other inline marks may run across lines; code
spans may not. Blank lines are kept, except at the start and end of a
message and around a heading.

## Editing the message you are writing

The compose box is a small text editor, the same in a terminal
emulator and on the console. The keys follow readline where the web
client has no key of its own. The text is stored as typed: the markdown
markers, `<@id>` mentions and `<:name:id>` custom emoji tokens are all
there, a custom emoji counting as one character for every movement and
deletion.

- **Moving.** **Left**/**Right** by character, **Ctrl+Left**/**Right**
  or **Alt+B**/**Alt+F** by word, **Home**/**End** or
  **Ctrl+A**/**Ctrl+E** to the ends of the line, **Ctrl+Home**/**End**
  to the ends of the text. **Up**/**Down** move between lines; **Up** on
  the first line leaves the box for the message list as before.
- **Lines.** **Shift+Enter**, **Alt+Enter** or **Ctrl+J** starts a new
  line (**Alt+Enter** is the one the Linux console can send, **Ctrl+J**
  the one xterm does not keep for itself). **Enter** sends. Pasted text
  keeps its line breaks.
- **Deleting.** **Backspace** (or **Ctrl+H**, which is the byte some
  terminals send for it) and **Delete** (or **Ctrl+D**) take the
  character before or after the cursor; **Ctrl+Backspace**,
  **Alt+Backspace** or **Ctrl+W** the word before, **Ctrl+Delete** or
  **Alt+D** the word after; **Alt+K** the rest of the line; **Ctrl+U**
  everything.
- **Selecting.** **Shift** with any movement key extends a selection,
  shown reversed. Where the terminal does not report Shift with the
  arrows (the console), **Ctrl+Space** sets a mark and the plain
  movement keys select from it; **Ctrl+Space** again or **Esc** drops
  it. Typing, **Backspace** or **Delete** replace or remove the
  selection.
- **Cut, copy, paste.** **Ctrl+C** copies and **Ctrl+X** cuts the
  selection into the client's own buffer, and onto the system clipboard
  through `wl-copy` (Wayland) or `xclip` (X11) when one is on PATH.
  **Alt+V** pastes that buffer. **Ctrl+V** reads the system clipboard:
  text goes in at the cursor, an image or files copied in a file manager
  are attached, as before. The terminal's own paste key works too.
- **Formatting.** **Ctrl+B** bold, **Ctrl+I** italic (or **Tab** while
  something is selected), **Alt+U** underline, **Ctrl+S**
  strikethrough, **Alt+C** inline code, **Alt+P** spoiler. They wrap the
  selection, or the word at the cursor; with nothing at the cursor they
  insert a pair to type into. The same key on already-marked text
  removes the marks, as in the web client.
- **Undo.** **Ctrl+Z** undoes, **Ctrl+Y** redoes. A run of typed
  characters up to a space is one step, so is a run of Backspaces;
  everything else is a step of its own. Sending clears the history.
- The autocompletes for `:`, `@` and `/` look at the text before the
  cursor and insert there, so they work in the middle of a message.
- The counter in the box's corner counts the whole text.

## Notifications

A direct message, or a message that mentions you (directly, through one
of your roles, or with @everyone unless you suppress those), is announced
outside the client by a program that does that:

- on a desktop, libnotify's `notify-send`;
- on a Linux console, GNU `mail`, when you choose it: the message is
  mailed to you locally, so the console's "You have new mail" is the
  notification, and `mail` reads it.

The **Notifications** row in the settings overlay (**F2**) picks the
mode: **Auto** (the default: notify-send where there is a display,
nothing elsewhere), **Desktop**, **Mail** or **Off**. Mail is never
picked on its own, since it puts messages in your mailbox: set the mode
to Mail for the console. The config file has the rest:

```toml
[ui]
notifications = "auto" # auto | desktop | mail (opt in) | off
notify_mail_to = "" # recipient; the login user when empty
notify_mail_command = "" # the mail program; "mail" when empty
notify_desktop_command = "" # the desktop program; "notify-send" when empty
notify_all_messages = false # also every message in channels set to all messages
notify_sound = true # a sound with every notification
notify_sound_file = "" # the sound; a built-in chime when empty
notify_sound_player = "" # the player; [media] audio_player or the usual ones when empty
```

Nothing is announced for your own messages, and a channel's own
notification settings (muted, mentions only) are respected. A program
that cannot be run is reported on the status line: `notify-send` comes
with libnotify and needs a notification daemon (mako, dunst, ...) to
show anything; `mail` comes with GNU mailutils and needs local mail
delivery.

**The channel you are reading is not announced while you can see it.**
A message in the open channel is on screen, so it sends no notification
and plays no sound as long as the terminal window has focus. When the
window is not in front (another window has focus, or, in console mode,
another VT is active), messages in the open channel are announced like
any other. The terminal reports focus changes through the usual focus
reporting escape (`CSI ? 1004 h`), which foot, kitty, alacritty,
wezterm, xterm and most others support; a terminal that does not report
focus is taken to be in front, so the open channel stays quiet there.
Under tmux, focus reaches the client only with `set -g focus-events on`.

**The sound** is played by the same kind of program that plays audio
attachments (see "Audio"): the bytes go to its stdin, it gets no
terminal, and so it works in a terminal emulator and on a Linux console
alike. With nothing configured, `[media] audio_player` is used, else the
first of mpv, ffplay, pw-play, paplay and aplay on PATH; a player of
your own goes in `notify_sound_player`. The sound is a short built-in
chime, or any file the player can play in `notify_sound_file` (`~/` is
expanded). A burst of messages plays it once, not once per message. The
**Notification sound** row in the settings overlay (**F2**) turns it
off. The sound follows the notification: with the mode Off there is
none; with Auto on a console, where nothing else is announced, the
sound is the whole notification.

## Interface overview

The UI has four **focus** areas, cycled with **Tab** / **Shift+Tab** (or **h**/**l** / **Left**/**Right**):

1. **Servers** - guild list and Direct Messages (`@me`).
2. **Channels** - channels for the selected server.
3. **Messages** - message history for the selected channel.
4. **Input** - compose box (text channels only, when you have permission to send).

A status line at the top shows gateway state, errors, and hints. Who is
typing in the channel is written into the compose box's bottom edge, the
way the web client puts it under the input. It takes no row of its own,
so a peer starting or stopping never moves the rest of the screen, and
it stays in view while you write your own message. **F2** turns typing
indicators off.

Others see you typing too, the way they see the web client's users: the
channel is told 1.5 seconds after your first key, again no sooner than
8 seconds later while keys keep coming, and no more once 10 seconds pass
without one or the message goes out. Nothing is sent for a slash command
or while you edit a message, and nothing at all with the **Show others
that you are typing** row of the settings overlay (**F2**) set to Off,
or in the config file:

```toml
[ui]
show_typing_indicators = true # who else is typing, in the compose box
send_typing = true # tell the channel when you are typing
```

Consecutive messages from one person are grouped under a single header,
so the message the pane opens on often began above its top row - a long
one you have scrolled into, or one grouped under a message that is now
off the pane - and nothing on the pane would say who wrote it. The
pane's title names them while that is so: `Messages ↑ someone`. It takes
none of the rows, so nothing that was on the pane is pushed off it, and
it goes again as soon as that message's own header comes into view.

Messages that concern you are highlighted the way the web app does it: a
message that mentions you (by name, through one of your roles, or with
@everyone/@here, unless the community's notification settings suppress
those) or that replies to one of your messages gets an amber bar in the
left margin, and a tinted background in the Fluxer theme. When the
selected message is a reply, the message it answers is marked with ↩ in
the margin, as long as it is loaded.

**p** (from the Servers, Channels or Messages focus) opens the **pings**
list: the messages that mentioned you, newest first, from every
community and direct message, as the web client's inbox shows them. Each
entry names the community and channel, the time, the author and the
start of the message. **↑** / **↓** move, **Enter** goes to the message:
the channel opens and the message is selected once its history is
loaded; when it is older than the loaded history, older pages are
fetched until it turns up. **x** takes an entry off the list and **X** all of them, on the
server too, so the web client agrees; **R** reloads; **Esc** or **q**
close. The profile of a selected message's author is on **u** now.

## Keyboard shortcuts

### Global (not typing in the input box)

| Key                     | Action                                                                                                                                                                                                                                                   |
| ----------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Tab**                 | Next focus (Servers → Channels → Messages → Input → …).                                                                                                                                                                                                  |
| **Shift+Tab**           | Previous focus.                                                                                                                                                                                                                                          |
| **Left** / **h**        | Previous focus.                                                                                                                                                                                                                                          |
| **Right** / **l**       | Next focus.                                                                                                                                                                                                                                              |
| **i**                   | Jump to **Input** (text channel, if you can send).                                                                                                                                                                                                       |
| **p**                   | **Pings**: the messages that mentioned you, newest first, from every community and direct message (see "Interface overview"). **↑** / **↓** move, **Enter** jumps to one, **x** dismisses it, **X** dismisses all, **R** reloads, **Esc** / **q** close. |
| **Enter**               | On a **link** channel: open URL in browser. On a **text** channel: jump to **Input**.                                                                                                                                                                    |
| **Esc**                 | Clear message selection; focus **Messages**.                                                                                                                                                                                                             |
| **↑** / **k**           | Move selection / scroll (depends on focus; see below).                                                                                                                                                                                                   |
| **↓** / **j**           | Move selection / scroll (depends on focus).                                                                                                                                                                                                              |
| **PageUp**              | Scroll message list up (larger step).                                                                                                                                                                                                                    |
| **PageDown**            | Scroll message list down (larger step).                                                                                                                                                                                                                  |
| **Ctrl+N** / **Ctrl+P** | Next / previous **text** channel (wraps; works from input too unless a popup is open).                                                                                                                                                                   |
| **Ctrl+K**              | Open **channel picker** (type to filter, **Enter** to jump).                                                                                                                                                                                             |
| **Alt+A**               | Jump to the **next channel** (after current) that has **unread** or **mention** badges; wraps.                                                                                                                                                           |
| **Alt+1** … **Alt+9**   | Go straight to a place in the left column: **Alt+1** is the conversation list, **Alt+2** the first community, and so on (see "Getting about").                                                                                                           |
| **Alt+↑** / **Alt+↓**   | Previous / next entry in the left column, from any focus; wraps.                                                                                                                                                                                        |
| **Alt+←** / **Alt+→**   | **Back** and **forward** through the channels you have visited. The bar says which way is open.                                                                                                                                                          |
| **Alt+L**               | Between the community you were **last** in and the conversation list.                                                                                                                                                                                   |
| **U**                   | Jump to the **new messages** line (see "The new messages line").                                                                                                                                                                                        |
| **Alt+M**               | **Member list** of the open channel, in a column beside the messages (see "The member list"). **Alt+J** / **Alt+K** scroll it. Closes itself in a direct message; not drawn below 80 columns.                                                            |

| **Alt+F**               | **Friends**: friends, the requests both ways, and the accounts you have blocked (see "Friends and blocking"). **←** / **→** switch group, **+** adds by tag, **Esc** closes.                                                                             |

| **Alt+P**               | **Pinned messages** of the open channel, newest pin first. **↑** / **↓** move, **Enter** jumps to one, **x** unpins it, **R** reloads, **Esc** / **q** close. Opening the list marks the channel's pins seen.                                            |
| **Alt+B**               | **Bookmarks**: the messages you have saved, from every community and direct message. **↑** / **↓** move, **Enter** jumps to one, **x** takes the bookmark off, **R** reloads, **Esc** / **q** close.                                                     |

| **/**                   | **Search messages** (see "Searching"). **←** / **→** pick the scope, **Enter** searches, then **↑** / **↓** move through the hits and **Enter** jumps to one.                                                                                            |

| **Alt+N**               | **Start a conversation** with somebody, or a group with several (see "Conversations").                                                                                                                                                                  |
| **Alt+G**               | Look after the **group** now open: rename it, add somebody, take somebody out, leave it.                                                                                                                                                                |

| **Alt+C**               | **Communities**: join with an invite, make one, browse the directory, list this community's invites or its webhooks, or leave it (see "Communities and invites").                                                                                                        |

| **Alt+V**               | **Voice**: join the open voice channel, ring a conversation, answer or turn down a call, mute, deafen, leave (see "Voice").                                                                                                                             |
| **F1**                  | **Keybindings** overlay - **↑** / **↓** / **PgUp** / **PgDn** scroll when it does not fit (**Esc** / **Enter** / **q** to close).                                                                                                                        |
| **Ctrl+H**              | Same overlay when focus is **not** the message input (in the input it is a **Backspace**, since that is the byte xterm's Backspace key sends).                                                                                                           |
| **F12**                 | **Debug panel**: session facts and the last log lines; **s** there writes them to a file, **f** maps the screen into the log (see "Debugging").                                                                                                          |
| **R**                   | **Refresh** active channel messages and guild metadata used for loads (clears local message cache for the channel and resets fetch/backoff state for the current guild).                                                                                 |
| **Ctrl+C**              | Quit - except with a **message selected** (it copies that message) or with **text selected in the input** (it copies the selection).                                                                                                                     |
| **Ctrl+L**              | Log out (clear intent flag) and quit - token is cleared when the process exits cleanly after this.                                                                                                                                                       |
| **q**                   | Quit.                                                                                                                                                                                                                                                    |

### When focus is **Servers**

| Key           | Action                      |
| ------------- | --------------------------- |
| **↑** / **k** | Previous server / DM entry. |
| **↓** / **j** | Next server / DM entry.     |

### When focus is **Channels**

| Key           | Action            |
| ------------- | ----------------- |
| **↑** / **k** | Previous channel. |
| **↓** / **j** | Next channel.     |

Changing channels marks read state for the new channel when applicable.

### When focus is **Messages**

| Key                | Action                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| ------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| **↑** / **k**      | With **message select mode** on: previous message; the pane scrolls to it once it would go off the top. Otherwise: scroll up a few lines.                                                                                                                                                                                                                                                                                                                    |
| **↓** / **j**      | With **message select mode** on: next message; the pane scrolls to it once it would go off the bottom. Otherwise: scroll down a few lines.                                                                                                                                                                                                                                                                                                                   |
| **s**              | **Select** mode: select the latest message (start of thread for reply / react / forward). The selected message carries its own header - the author's name and the time - even where it is grouped under another message from the same person, so whoever wrote it is named however far you have scrolled. Their picture comes with it only where the header that already names them has gone off the top of the pane, so the same face is never drawn twice. |
| **G**              | Jump to the **newest** message (also from the Servers and Channels focus). While you are scrolled up, the view stays on the message you are reading as new messages arrive or older ones load.                                                                                                                                                                                                                                                               |
| **y** / **Ctrl+C** | **Copy** the selected message: its text, and the link of each file on it, one per line. It goes to the system clipboard through `wl-copy` (Wayland) or `xclip` (X11) when one of them is there, and always to the client's own cut buffer, so **Alt+V** in the input pastes it - that is the way on the Linux console, where there is no clipboard.                                                                                                          |
| **r**              | **Reply** to the selected message (only in select mode). Moves focus to **Input** with reply state set.                                                                                                                                                                                                                                                                                                                                                      |
| **u**              | **Profile** of the selected message's author: name, badges, pronouns, bio, when they joined Fluxer and the community, roles, connections, mutual communities and friends. **↑** / **↓** scroll; **p** shows the profile picture full size; **Esc** / **q** close.                                                                                                                                                                                            |
| **e**              | **React**: pick an emoji (**Enter** sends the reaction via API; **Esc** cancels).                                                                                                                                                                                                                                                                                                                                                                            |
| **f**              | **Forward**: optional note, switch target channel (**Ctrl+K** or list), **Enter** to send (reference type forward).                                                                                                                                                                                                                                                                                                                                          |
| **Ctrl+E**         | **Edit** the selected message (your messages only; **Enter** in input to save, **Esc** to cancel).                                                                                                                                                                                                                                                                                                                                                           |
| **Ctrl+D**         | **Delete** the selected message (yours, or with **Manage Messages**).                                                                                                                                                                                                                                                                                                                                                                                        |
| **a**              | **Message actions**: every action the selected message allows, in one menu (see "Message actions"). **↑** / **↓** move, **Enter** chooses, **Esc** steps back out of a list the menu led to, or closes it.                                                                                                                                                                                                                                                    |
| **Y**              | **Copy a link** to the selected message - the address the web app opens it at. Needs the instance to have said where its web app is (it comes with the login).                                                                                                                                                                                                                                                                                                |
| **P**              | **Pin** the selected message to the channel, or **unpin** it. Your own message anywhere you can post; anybody's with **Manage Messages**.                                                                                                                                                                                                                                                                                                                     |
| **b**              | **Bookmark** the selected message, or take the bookmark off. **Alt+B** lists them.                                                                                                                                                                                                                                                                                                                                                                            |
| **v**              | **Who reacted** to the selected message. **←** / **→** walk its other reactions without closing, **↑** / **↓** scroll, **x** clears everybody's reaction with the one shown (**Manage Messages**), **Esc** / **q** close.                                                                                                                                                                                                                                     |
| **m**              | **Mark** the selected message; marked messages carry a red cross in the margin. The menu's "Delete the marked messages" then deletes every marked message of the channel in one call. Needs **Manage Messages**, at least two marks, and the server refuses messages more than two weeks old.                                                                                                                                                                 |
| **[**              | Load **older messages** (prepends history; repeat until exhausted).                                                                                                                                                                                                                                                                                                                                                                                          |

Edited messages show **(edited)** in dim italics after the timestamp when the API supplies `edited_timestamp` (including live **MESSAGE_UPDATE** from the gateway).

### When focus is **Input**

| Key                                                                                               | Action                                                                                                                                                                                                  |
| ------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Enter**                                                                                         | Send message, save **edit**, or forward with reference only. Long lines **wrap** and the input bar **grows** with the text, then scrolls to the cursor.                                                 |
| **Shift+Enter** / **Alt+Enter** / **Ctrl+J**                                                      | New line. **Ctrl+J** is the one that arrives in xterm, which binds Alt+Enter to its own fullscreen action.                                                                                              |
| **←** / **→**, **Ctrl+←** / **Ctrl+→** (or **Alt+B** / **Alt+F**)                                 | Move by character or word; **←** with nothing before the cursor leaves **Input** and focuses **Messages**, so **←** walks the boxes back the way **→** walks them forward. **Ctrl+←** stays in the box. |
| **Home** / **End** (or **Ctrl+A** / **Ctrl+E**), **Ctrl+Home** / **Ctrl+End**                     | Start or end of the line; of the whole text.                                                                                                                                                            |
| **↑** / **↓**                                                                                     | Line above or below; **↑** on the first line leaves **Input** and focuses **Messages**.                                                                                                                 |
| **Backspace** (or **Ctrl+H**) / **Delete** (or **Ctrl+D**)                                        | Delete the character before / after the cursor (or the selection).                                                                                                                                      |
| **Ctrl+Backspace** / **Alt+Backspace** / **Ctrl+W**, **Ctrl+Delete** / **Alt+D**                  | Delete the previous / next whitespace-separated word.                                                                                                                                                   |
| **Alt+K**                                                                                         | Delete to the end of the line.                                                                                                                                                                          |
| **Shift+movement**, or **Ctrl+Space** then movement                                               | Select; **Esc** drops the selection.                                                                                                                                                                    |
| **Ctrl+C** / **Ctrl+X** / **Alt+V**                                                               | Copy / cut the selected text here (also to `wl-copy` or `xclip`) / paste back what was cut or copied, a message copied with **y** included.                                                             |
| **Ctrl+V**                                                                                        | Paste text from the system clipboard at the cursor; an image or files on it are attached instead.                                                                                                       |
| **Alt+S**                                                                                         | Open the sticker picker (also `/sticker`, `/sticker <name>`).                                                                                                                                           |
| **Ctrl+X** (nothing selected)                                                                     | Drop the last staged sticker, or the last staged file when no sticker is staged.                                                                                                                        |
| **Ctrl+B**, **Ctrl+I** (or **Tab** with a selection), **Alt+U**, **Ctrl+S**, **Alt+C**, **Alt+P** | Bold, italic, underline, strikethrough, code, spoiler around the selection or the word at the cursor; again to remove.                                                                                  |
| **Ctrl+Z** / **Ctrl+Y**                                                                           | Undo / redo.                                                                                                                                                                                            |
| `/debug`, `/debug save`, `/debug frame`                                                           | Debug panel; write its facts and log lines to a file; map the screen into the log (see "Debugging").                                                                                                    |
| **Ctrl+U**                                                                                        | Clear the whole input line.                                                                                                                                                                             |
| **Esc**                                                                                           | If replying/forwarding, cancel; if picking a reaction, cancel; otherwise leave **Input** and focus **Messages**.                                                                                        |
| **:** (colon)                                                                                     | Start **custom emoji** autocomplete (server emojis + unicode picker).                                                                                                                                   |
| **@**                                                                                             | Start **@mention** autocomplete (users/roles in guilds; DMs use recipients). Triggers loading full member list from the API only when needed.                                                           |

Plain letters (without **Ctrl**) are inserted into the message, except where autocomplete consumes them.

### @mention autocomplete (while open)

| Key                 | Action                            |
| ------------------- | --------------------------------- |
| **↑** / **↓**       | Previous / next suggestion.       |
| **Tab** / **Enter** | Insert selected mention.          |
| **Esc**             | Close autocomplete.               |
| **Backspace**       | Edit text; filter updates.        |
| **Any character**   | Type to filter (unless **Ctrl**). |

### Sticker picker (while open)

| Key                                                                   | Action                                                    |
| --------------------------------------------------------------------- | --------------------------------------------------------- |
| **j** / **k**, **↑** / **↓**                                          | Move through the stickers.                                |
| **g** / **G**, **Home** / **End**                                     | First / last sticker.                                     |
| **Ctrl+D** / **Ctrl+U**, **Ctrl+F** / **Ctrl+B**, **PgDn** / **PgUp** | Move by ten.                                              |
| **Enter**, **l** / **→**                                              | Put the sticker on the message (at most three) and close. |
| **q**, **h** / **←**, **Esc**                                         | Close the picker.                                         |
| **/**                                                                 | Search by name or tag; the list narrows as you type.      |
| **Enter** (searching)                                                 | Keep the search and go back to moving.                    |
| **Esc** (searching)                                                   | Leave the search; the list goes back as it was.           |
| **Backspace** (**Ctrl+U**)                                            | While searching: edit (clear) what is typed.              |

### Emoji autocomplete (while open)

| Key                 | Action                                                                                         |
| ------------------- | ---------------------------------------------------------------------------------------------- |
| **↑** / **↓**       | Previous / next emoji.                                                                         |
| **Tab** / **Enter** | Insert selected emoji into the message, **or** confirm **reaction** when **e** flow is active. |
| **Esc**             | Close autocomplete.                                                                            |
| **Backspace**       | Edit; filter updates.                                                                          |
| **Any character**   | Type to filter (unless **Ctrl**).                                                              |

---

## Tips

- **Capital R** is refresh; **lowercase r** in message select mode is reply.
- Message **select mode** is only active after **s** in the **Messages** focus.
- **Performance mode** (**F2** → Performance Mode, or `performance_mode =
  true` under `[ui]`) is for slow machines. It draws no pictures at all
  (profile pictures, previews, custom emoji as pictures; **Ctrl+O** still
  opens one on request), no animations and no typing indicators, ticks
  twice a second instead of ten times, and draws a burst of gateway
  traffic (presence changes in a big community, say) once every 200 ms
  instead of once per event. Your own keys are always drawn at once.

## Debugging

When something goes wrong, the client can say what it saw:

- **`/debug`** (or **F12**) opens the debug panel: version, terminal
  and picture protocol, cell size, gateway state, what is loaded, how
  long the last frame took, where the log is, and the last few hundred
  log lines. The **debug log** row names the file in use, `--debug-log`
  or not, and when no log is being kept it names the one `--debug`
  would write. **s** in the panel, or `/debug save`, writes all of that
  to a file whose path shows in the status line; attach it to a bug
  report.
- **`/debug frame`**, or **f** in the panel, puts a map of the screen
  into the log for a layout bug ("the text is not where it should
  be"): one character per cell, `#` for a border, `P` for a picture,
  `e` for a custom emoji, `T` for text, blank for nothing, with a line
  above it saying how many rows the compose box got and how many it
  asked for, and where the cursor is. The panel closes first, so the
  map is of what was under it. The map says where text is, not what it
  says: single blanks between words are filled in, so not even word
  lengths are in it. `/debug save` afterwards keeps it with the rest.
- **`--debug`** keeps the whole log from start to exit, panics
  included, in `$XDG_STATE_HOME/fluxer-tui/debug.log` (that is
  `~/.local/state/fluxer-tui/debug.log` as a rule; `--help` says so too,
  and the debug panel shows the path in use). `--debug-log FILE` names
  the file. `FLUXER_TUI_DEBUG=1` (or a path) does the same from
  the environment, which is handy with `nix run`. The folder is made
  on every start, so it is there to look in; the file only appears
  when a log is kept, and `/debug save` snapshots land beside it.
- **`--no-graphics-query`** (or `FLUXER_TUI_NO_GRAPHICS_QUERY=1`) skips
  asking the terminal which picture protocol it speaks. Under tmux,
  expect or another program driving the client, nothing answers, and
  the thread waiting for the answer eats every other keystroke.

The log respects your privacy: it records what happened, not what was
said. Gateway events appear as their kind, size and shape (field names
and types, string lengths, the ids of channels, communities, users and
messages); HTTP requests as method, path, status, duration and whether they went
over IPv4 or IPv6 (the server counts login attempts per address, and a
browser and a client on the same machine can take different routes),
with the API's error code when there is one; pictures as host, size and result;
plus the client's own status-line messages, errors and frame timings.
It never contains message text, user or community names, e-mail
addresses, file names or paths (a status line that mentions one is
logged with `<file>` or `<path>` in its place), notification text, or
the token. The log's own path is the one path shown on screen, in the
debug panel, and a snapshot saved from that panel scrubs it to `<path>`
like any other, so a file you attach to a bug report does not say where
your home directory is. Events that come in bursts, presence changes for one, are
logged once and then every hundredth time.

## Console mode (Linux VT, no terminal emulator)

On a plain Linux virtual console there is no terminal emulator to draw
pictures, so fluxter paints its own screen through DRM/KMS: text with a
real font, colour emoji, custom emoji, animated emoji, image previews and
GIFs, all on the console. It switches on by itself when `TERM=linux` and
stdin is a VT (a getty login on tty7, say), and needs:

- access to the DRM device: membership of the `video` group, or an active
  logind session on that VT (which grants it);
- a text font and an emoji font, found through `fc-match monospace` and
  `fc-match emoji` unless set explicitly.

Keys come from the VT as usual. Switching VTs (Alt+Fn) hands the display
over and back. Settings, all optional:

```toml
[console]
mode = "auto" # auto | always | never
drm_device = "" # default /dev/dri/card0
font = "" # path to a TTF/OTF; default: fc-match monospace
bold_font = "" # default: fc-match monospace:bold
emoji_font = "" # default: fc-match emoji (e.g. Noto Color Emoji)
font_px = 28 # text size in pixels
```

`FLUXER_TUI_CONSOLE=never|always` overrides the mode for one run, and
`FLUXER_TUI_CONSOLE=dump:/some/dir:1280x720` renders frames to PPM files
instead of a display, which is how the console renderer is tested.

## Known issues & TODOs

- **Voice** is view-only: the client shows who is in a voice channel but
  cannot join, transmit or hear. Fluxer's voice runs over WebRTC through
  LiveKit, which would mean a whole WebRTC stack in the client.
- Some communities answer the member list request with a gateway
  timeout (504) from the server's own member service. The client keeps
  the pages that arrived, says in plain words that the list is
  unavailable, and offers @mentions from the members it has seen.
- Open a **feature request** issue for anything you want that is not here yet.

## Security

The client holds a login token and talks to the network, so two checks
run on GitHub from `.github/workflows/security.yml`:

- **Dependency advisories.** `cargo audit` reads `Cargo.lock` against the
  [RustSec](https://rustsec.org) database, on every push and pull request
  that touches the sources, `Cargo.toml` or `Cargo.lock`, and again every
  Monday, since an advisory can be published against a crate that has not
  changed here in months. A vulnerability fails the run. Unmaintained,
  unsound and yanked crates are listed without failing it: several of them
  are reached through `ratatui` and `image` and cannot be upgraded from
  here.
- **CodeQL** reads the sources and files what it finds under the
  repository's Security tab.

Both also run on demand from the Actions tab. To audit a checkout
yourself:

```bash
cargo install cargo-audit --locked
cargo audit
```

## License

Copyright (C) 2026 polonius-dev and the fluxter contributors.

This program is free software: you can redistribute it and/or modify it
under the terms of the GNU General Public License as published by the
Free Software Foundation, either version 3 of the License, or (at your
option) any later version. It is distributed in the hope that it will be
useful, but WITHOUT ANY WARRANTY, without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the GNU General
Public License in `LICENSE` for details.

### Why there are two license files

This client started from [dogbonewish/fluxer-tui](https://github.com/dogbonewish/fluxer-tui),
which is MIT-licensed, and the MIT license permits redistribution under
stricter terms as long as its notice is kept. Developing separately
changes nothing about that: the obligation follows the code, not the
relationship between the repositories.

- Everything that was already in the upstream repository when this one
  branched off (upstream commit `dc78802`, 2026-04-13) stays under the
  MIT license. Its notice is in `LICENSE-MIT` and must travel with every
  copy of this program, source or binary.
- Everything added since then, the commits that
  `git log dc78802..master` lists, is licensed GPL-3.0-or-later. One
  outside contribution made while this repository was still labelled MIT
  keeps the terms it was offered under.

The combined work is therefore distributed under the GPL: use it, modify
it and redistribute it under those terms, keeping both notices.
