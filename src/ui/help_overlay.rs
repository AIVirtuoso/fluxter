use crate::app::App;
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

const HELP: &str = r#"Global (almost any screen)
  F1 - keybindings (this overlay)
  F2 - settings (UI preferences; saved to config)
  Ctrl+H - keybindings when focus is not the message input
  Ctrl+C - quit (it copies instead when a message or input text is selected)
  Ctrl+L - log out and quit
  q - quit (when not typing in input)

Focus & navigation
  Tab / Shift+Tab - cycle: servers → channels → messages → input
  h / l / Left / Right - same as Tab (previous / next focus)
  Alt+1 … Alt+9 - go straight to a place in the left column: Alt+1 is the
           conversation list, Alt+2 the first community, and so on to Alt+9
  Alt+Up / Alt+Down - previous / next entry in that column, from any focus,
           wrapping; no need to put the focus on it first
  Alt+Left / Alt+Right - back and forward through the channels you have
           visited, the way a browser's do. The bar says which way is open.
  Alt+L - back and forth between the community you were last in and the
           conversation list
  U - jump to the "new messages" line (see Other)
  Esc - messages focus; clears message selection; closes overlays
  i - jump to input (text channel with send permission)
  Enter (channels) - open messages, or open link-channel URL, or focus input on text channel
  R - refresh: reload current channel messages and guild channels/members
  p - pings: the messages that mentioned you (Enter jumps to one, x dismisses it, X all)
  Alt+M - the member list of the open channel, in a column beside the messages;
           Alt+J / Alt+K scroll it. It follows you from channel to channel and
           closes itself in a direct message, which has no member list.
  Alt+F - friends: the four groups (friends, wanting, asked, blocked). Left/Right
           switch group, + adds by tag (name#0001), Enter opens the conversation,
           n gives a friend a name of your own, a accepts an incoming request,
           x undoes whichever tie there is, B blocks, R reloads.
  Alt+P - the pinned messages of the open channel (Enter jumps to one, x unpins)
  Alt+B - the messages you have bookmarked, from everywhere (Enter jumps, x removes)
  / - search messages. Type the query, Left/Right pick the scope (this channel,
           this community, everywhere), Enter searches. Then Up/Down move
           through the hits, Enter jumps to one, n/p turn the page, / goes back
           to the query, Esc closes. A query takes "words in quotes" that have
           to appear together, from:someone, has:image|sound|video|file|embed
           and pinned:true.
  Alt+N - start a conversation: type to filter the people the client knows,
           Space picks several for a group, Enter opens it (or makes the group)
  Alt+G - the group conversation now open: rename it, add somebody, take
           somebody out, leave it
  Alt+C - communities: join with an invite (a code or a pasted link; what it
           leads to is shown before you take it), make one, browse the
           directory, list this community's invites (y copies a link, + makes
           one to the open channel, x revokes) or leave it
  Alt+V - voice: join the open voice channel, ring a conversation, answer or turn
           down a call, mute, deafen, leave. The client joins and keeps the
           bookkeeping; the sound itself is carried by the program named in
           [media] voice_command, and the menu says plainly when there is none
           (see README, "Voice").

Your account
  Alt+Z - where you are signed in: every live session, what it is, where it was
           last seen and when. R reloads. Ending one needs your password, which
           this client never holds, so that is done in the web client.

Servers (left column)
  Up / Down / j / k - move server selection
  n - notification settings for the selected community
  l / Right - open channel list for selected server

Channels (middle column)
  Up / Down / j / k - move channel
  n - notification settings for the selected community
  Enter - open message view for channel
  In the direct messages list: P keeps a conversation at the top of the list
           (it shows ·pin), x closes it. Closing deletes nothing; the
           conversation comes back the moment either side writes.

GIFs
  /gif - the GIF picker: alone for what is trending, /gif <words> to search.
           Type to change the search, Enter runs it, up/down move, Enter sends
           the one under the cursor, Esc closes.

Messages
  Up / Down / j / k - scroll list, or move selection when a message is selected
  PgUp / PgDn - scroll message pane
  G - jump to the newest message; while scrolled up, the view stays put as messages arrive
  Scroll up near the top - older messages load automatically
  s - select last message (selection mode)
  a - everything that can be done with the selected message, in one menu (the
           keyboard's answer to the web client's right-click): pin, bookmark, mark
           unread, mark the channel or the community read, hide or show the link
           previews, who reacted, clear the reactions, copy a link or the id,
           remove one file from it, delete the marked messages, report it.
           ↑/↓ move, Enter chooses, Esc steps back out of a list or closes.
           The keys below are the same things without the menu.
  y or Ctrl+C - copy the selected message: its text and the links of its files, to
           the system clipboard where wl-copy or xclip is there, and always to the
           cut buffer, so Alt+V pastes it in the input (that is the way on the console)
  Y - copy a link to the selected message (the web app address of it)
  P - pin the selected message to the channel, or unpin it
  b - bookmark the selected message, or take the bookmark off
  v - who reacted to the selected message (←/→ walks its other reactions, x clears
           everybody's reaction with the one shown, with Manage Messages)
  m - mark the selected message; a → "Delete the marked messages" deletes every
           marked message of the channel in one call (needs Manage Messages, and
           the server refuses messages older than two weeks)
  r - reply to selected message
  u - profile of the selected message's author. p shows the picture full size,
           Esc closes, and + / x / B act on how you stand with them: + asks or
           accepts, x unfriends, turns down, takes back or unblocks, B blocks.
           The line at the bottom of the profile names whichever of the three
           apply, so there is no guessing which. n writes your own private note
           about them: Enter saves it, an empty one deletes it, Esc cancels.
  f - forward selected (pick channel with Ctrl+K, optional note, Enter)
  e - react: opens emoji picker on selected message (Enter to send reaction, Esc cancels)
  Ctrl+E - edit your message (focuses input; Enter save, Esc cancel)
  Ctrl+D - delete selected (yours, or mod with Manage Messages)
  Ctrl+O - the selected message's picture or GIF full size in-terminal (the chat shows
           previews); videos open via the system default app; audio plays through
           mpv/ffplay/pw-play/paplay/aplay or [media] audio_player (again: stop)

Input
  Enter - send; save edit; send forward with reference
  Shift+Enter / Alt+Enter / Ctrl+J - new line (Alt+Enter is the one the
           console sends, Ctrl+J the one xterm does not keep for itself)
  Left/Right, Home/End (Ctrl+A/Ctrl+E), Ctrl+Left/Right or Alt+B/Alt+F - move by
           character, line, word; Ctrl+Home/End - start/end of the text
  Up/Down - line above/below; Up on the first line leaves for the messages
  Backspace (or Ctrl+H) / Delete (or Ctrl+D) - delete before / after the cursor
  Ctrl+Backspace, Alt+Backspace, Ctrl+W / Ctrl+Delete, Alt+D - delete the word
           before / after the cursor
  Alt+K - delete to the end of the line     Ctrl+U - clear input
  Shift+arrows, or Ctrl+Space then arrows - select; Esc drops the selection
  Ctrl+C / Ctrl+X - copy / cut the selected text here (also to wl-copy or xclip)
  Alt+V - paste what was cut or copied here; Ctrl+V - the system clipboard
  Ctrl+B bold  Ctrl+I italic  Alt+U underline  Ctrl+S strike  Alt+C code  Alt+P spoiler
           - around the selection or the word at the cursor; again removes them
  Ctrl+Z / Ctrl+Y - undo / redo (a typed word or a run of Backspaces is one step)
  Ctrl+F or /attach - file picker: any kind of file, with a preview of pictures and videos
  /attach <path> - attach a file by path; Ctrl+V - paste text from the clipboard, or
           attach the image or files on it
  Ctrl+O - the last staged picture or video full size; Ctrl+X - drop the last staged
           sticker, or the last staged file when no sticker is staged
  Alt+S or /sticker - sticker picker: j/k move, / searches by name or tag, Enter puts
           one on the message (at most 3), /sticker <name> opens it filtered
  Enter sends text, attachments and stickers together (the compose box shows what is
           staged)
  /status online|idle|dnd|invisible - your own online status (the status bar
           shows it); /customstatus <text> sets the line under your name,
           /customstatus alone clears it
  : - custom emoji autocomplete     @ - mention autocomplete (guild/DM)
  Long lines wrap; input height grows with wrapped rows

Sticker picker (Alt+S, /sticker)
  j / k (or Up/Down) - move        g / G - first / last
  Ctrl+D / Ctrl+U, Ctrl+F / Ctrl+B, PgDn / PgUp - by ten
  Enter or l - put the sticker on the message    q, h or Esc - close
  / - search by name or tag: type to narrow the list, Enter keeps the search,
      Esc leaves it and puts the list back as it was, Backspace edits it
      (Ctrl+Backspace or Ctrl+U clears it)
  Stickers of other communities are listed under their name; sending one needs the
  right to use external stickers.

Channel picker (Ctrl+K)
  Type to filter   Up/Down - move   Enter - jump   Esc - close   Backspace

Ctrl+channel (disabled while : or @ autocomplete is open)
  Ctrl+N / Ctrl+P - next / previous text channel (wraps)
  Ctrl+K - channel picker
  Ctrl+E / Ctrl+D - edit / delete selected message (messages focus + selection)
  Ctrl+O - full-size picture when a message is selected (see Messages)

Alt (outside the compose box, where these letters edit the text instead)
  Alt+A - next channel with unread or mention (hotlist; wraps)
  Alt+1-9 - a place in the left column   Alt+Up/Down - step through it
  Alt+Left/Right - back / forward through the channels you have visited
  Alt+L - the last community, or the conversation list
  Alt+S - sticker picker (see Input)
  Alt+M - member list                 Alt+J / Alt+K - scroll it
  Alt+F - friends and blocked people  Alt+B - your bookmarks
  Alt+P - the channel's pinned messages
  Alt+N - start a conversation        Alt+G - look after the open group
  Alt+C - join, make, browse or leave a community
  Alt+V - voice: join, answer, mute, leave
  /     - search messages

Other
  The "new messages" line: an amber rule across the pane above the first
           message that arrived since you last opened the channel. U jumps to
           it. It stays where it was while you read, rather than sliding down
           as the client marks things read, and goes once you are at the
           bottom with nothing unread. A channel you have never opened gets
           no line, since a rule above the whole history says nothing.
  What you can press: the bar under the title says it for wherever the
           focus is, and every overlay — profile, friends, pins, pickers, the
           settings, this one — says it on its own bottom line, naming only
           what would actually do something where you are. The : @ and /
           popups have no row to spare, so their keys are on their title;
           the compose box says how to finish and how to leave whichever
           mode it is in. When something happens (a request sent, a message
           pinned) the words take that line for a few seconds and then the
           keys come back.
  Who is about: a filled circle in front of a name, green online, amber idle,
           red do not disturb; nothing at all where the server has said
           nothing. A one-to-one conversation shows the other person's in
           place of its @. u on a message spells the state out in words.
  Edited messages show “(edited)” after the timestamp when the API sends edited_timestamp.
  F12 or /debug - debug panel: session facts and the last log lines (no message text,
           no names); s there, or /debug save, writes them to a file for a bug report;
           f there, or /debug frame, puts a map of the screen (where borders, pictures
           and text are, not the words) into the log for a layout bug.
           Start with --debug to keep the whole log in a file (see README, "Debugging").
"#;

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    frame.render_widget(Clear, area);
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(4),
            Constraint::Length(1),
        ])
        .split(area);

    let mid = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(20),
            Constraint::Length(2),
        ])
        .split(outer[1]);

    let popup = mid[1];
    frame.render_widget(Clear, popup);

    let body = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(popup);

    let content = body[0];
    let text_style = Style::default().fg(crate::ui::theme::text());
    let lines: Vec<Line> = HELP
        .lines()
        .map(|l| Line::from(Span::styled(l.to_string(), text_style)))
        .collect();
    let help_text = Text::from(lines);

    let inner_w = content.width.saturating_sub(2).max(1);
    let inner_h = content.height.saturating_sub(2).max(1);

    let line_total = Paragraph::new(help_text.clone())
        .wrap(Wrap { trim: true })
        .line_count(inner_w)
        .max(1) as u16;
    let max_scroll = line_total.saturating_sub(inner_h);
    let scroll_y = app.help_scroll.min(max_scroll);

    let block = Block::default()
        .title(Line::from(Span::styled(
            " Keybindings ",
            Style::default()
                .fg(crate::ui::theme::accent())
                .add_modifier(Modifier::BOLD),
        )))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(crate::ui::theme::accent_dim()));

    let paragraph = Paragraph::new(help_text)
        .block(block)
        .wrap(Wrap { trim: true })
        .scroll((scroll_y, 0))
        .alignment(Alignment::Left);

    frame.render_widget(paragraph, content);

    let footer = if max_scroll > 0 {
        " ↑/↓ PgUp/PgDn - scroll · Esc / Enter / q - close "
    } else {
        " Esc / Enter / q - close "
    };

    crate::ui::footer::render(frame, body[1], app, footer);
}
