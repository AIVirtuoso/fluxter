package main

// The desktop portal's ScreenCast: what a browser uses to share a screen
// on Wayland. The portal (xdg-desktop-portal-wlr here) shows its own
// chooser for the output or the window, and hands back a PipeWire
// stream, which a capture program then encodes.

import (
	"errors"
	"fmt"
	"os"
	"strings"
	"sync/atomic"
	"time"

	"github.com/godbus/dbus/v5"
)

const (
	portalName   = "org.freedesktop.portal.Desktop"
	portalPath   = "/org/freedesktop/portal/desktop"
	screenCastIf = "org.freedesktop.portal.ScreenCast"
	requestIf    = "org.freedesktop.portal.Request"
	sessionIf    = "org.freedesktop.portal.Session"
)

// A screen the portal agreed to share: the PipeWire node, the connection
// to reach it through, and the session to close when done.
type screenCast struct {
	conn    *dbus.Conn
	session dbus.ObjectPath
	node    uint32
	pipe    *os.File
}

var portalToken atomic.Uint32

// Ask for a screen. This blocks while the portal's chooser is up, and
// fails cleanly when the user dismisses it.
func openScreenCast() (*screenCast, error) {
	conn, err := dbus.SessionBus()
	if err != nil {
		return nil, fmt.Errorf("session bus: %w", err)
	}
	portal := conn.Object(portalName, portalPath)

	results, err := portalRequest(conn, portal, screenCastIf+".CreateSession", map[string]dbus.Variant{
		"session_handle_token": dbus.MakeVariant(newToken()),
	})
	if err != nil {
		return nil, fmt.Errorf("create session: %w", err)
	}
	sessionValue, ok := results["session_handle"]
	if !ok {
		return nil, errors.New("create session: no session handle")
	}
	var session dbus.ObjectPath
	switch v := sessionValue.Value().(type) {
	case string:
		session = dbus.ObjectPath(v)
	case dbus.ObjectPath:
		session = v
	default:
		return nil, errors.New("create session: unreadable session handle")
	}
	sc := &screenCast{conn: conn, session: session}

	// monitors and windows both, one at a time, cursor drawn into the
	// picture (2); the portal's own chooser decides which
	if _, err := portalRequest(conn, portal, screenCastIf+".SelectSources", map[string]dbus.Variant{
		"types":       dbus.MakeVariant(uint32(3)),
		"multiple":    dbus.MakeVariant(false),
		"cursor_mode": dbus.MakeVariant(uint32(2)),
	}, session); err != nil {
		sc.close()
		return nil, fmt.Errorf("select sources: %w", err)
	}

	results, err = portalRequest(conn, portal, screenCastIf+".Start", map[string]dbus.Variant{}, session, "")
	if err != nil {
		sc.close()
		return nil, fmt.Errorf("start: %w", err)
	}
	node, err := firstStreamNode(results["streams"])
	if err != nil {
		sc.close()
		return nil, err
	}
	sc.node = node

	var fd dbus.UnixFD
	if err := portal.Call(screenCastIf+".OpenPipeWireRemote", 0, session, map[string]dbus.Variant{}).Store(&fd); err != nil {
		sc.close()
		return nil, fmt.Errorf("open pipewire remote: %w", err)
	}
	sc.pipe = os.NewFile(uintptr(fd), "pipewire")
	return sc, nil
}

func (sc *screenCast) close() {
	if sc.pipe != nil {
		_ = sc.pipe.Close()
		sc.pipe = nil
	}
	if sc.session != "" {
		_ = sc.conn.Object(portalName, sc.session).Call(sessionIf+".Close", 0).Err
		sc.session = ""
	}
}

// One portal call in the request/response shape: the method returns a
// request object at once and the answer arrives as a Response signal on
// it. The signal is subscribed to before the call so it cannot be
// missed. `args` go before the options map.
func portalRequest(conn *dbus.Conn, portal dbus.BusObject, method string, options map[string]dbus.Variant, args ...any) (map[string]dbus.Variant, error) {
	token := newToken()
	options["handle_token"] = dbus.MakeVariant(token)
	sender := strings.TrimPrefix(conn.Names()[0], ":")
	sender = strings.ReplaceAll(sender, ".", "_")
	expected := dbus.ObjectPath(fmt.Sprintf("%s/request/%s/%s", portalPath, sender, token))

	signals := make(chan *dbus.Signal, 16)
	conn.Signal(signals)
	defer conn.RemoveSignal(signals)
	match := []dbus.MatchOption{
		dbus.WithMatchObjectPath(expected),
		dbus.WithMatchInterface(requestIf),
		dbus.WithMatchMember("Response"),
	}
	if err := conn.AddMatchSignal(match...); err != nil {
		return nil, err
	}
	defer func() { _ = conn.RemoveMatchSignal(match...) }()

	callArgs := append(append([]any{}, args...), options)
	var handle dbus.ObjectPath
	if err := portal.Call(method, 0, callArgs...).Store(&handle); err != nil {
		return nil, err
	}
	if handle != expected {
		// an older portal names the request itself
		extra := []dbus.MatchOption{
			dbus.WithMatchObjectPath(handle),
			dbus.WithMatchInterface(requestIf),
			dbus.WithMatchMember("Response"),
		}
		if err := conn.AddMatchSignal(extra...); err != nil {
			return nil, err
		}
		defer func() { _ = conn.RemoveMatchSignal(extra...) }()
	}

	// the chooser can stay up for as long as the user takes; two
	// minutes is the point at which nobody is answering
	timeout := time.After(2 * time.Minute)
	for {
		select {
		case sig := <-signals:
			if sig == nil || (sig.Path != expected && sig.Path != handle) || len(sig.Body) < 2 {
				continue
			}
			code, _ := sig.Body[0].(uint32)
			results, _ := sig.Body[1].(map[string]dbus.Variant)
			switch code {
			case 0:
				if results == nil {
					results = map[string]dbus.Variant{}
				}
				return results, nil
			case 1:
				return nil, errors.New("cancelled in the chooser")
			default:
				return nil, fmt.Errorf("the portal answered %d", code)
			}
		case <-timeout:
			return nil, errors.New("no answer from the portal")
		}
	}
}

// The node id of the first stream in a Start response, whose shape is
// a(ua{sv}).
func firstStreamNode(streams dbus.Variant) (uint32, error) {
	list, ok := streams.Value().([][]any)
	if !ok || len(list) == 0 || len(list[0]) == 0 {
		return 0, errors.New("the portal named no stream")
	}
	node, ok := list[0][0].(uint32)
	if !ok {
		return 0, errors.New("the portal's stream has no node id")
	}
	return node, nil
}

func newToken() string {
	return fmt.Sprintf("fluxter%d_%d", os.Getpid(), portalToken.Add(1))
}
