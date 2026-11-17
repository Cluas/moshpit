/*
 * Claude Code 2.1.326 terminal stage program for screenshot and UI test fixtures.
 *
 * Replays either:
 *   claude                   Edit confirmation dialog + persistent bottom composer
 *   claude --working <file>  Active turn log from <file> + persistent bottom composer
 *
 * Designed to keep the 4-row composer docked to the bottom edge across every
 * tmux pane size (SIGWINCH) and reset pending composer drafts cleanly on Esc,
 * Ctrl-U, Ctrl-C, Enter, or a 10s idle timeout.
 */

#include <ctype.h>
#include <errno.h>
#include <fcntl.h>
#include <poll.h>
#include <signal.h>
#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <termios.h>
#include <time.h>
#include <unistd.h>

#define MAX_LINES 256
#define MAX_SEGS 12
#define MAX_DRAFT 255
#define IDLE_RESET_MS 10000

/* 24-bit TrueColor ANSI SGR foreground/background codes matching dark theme */
#define FG_DEFAULT   "\033[39m"
#define FG_WHITE     "\033[38;2;242;242;242m"
#define FG_CLAWD     "\033[38;2;215;119;87m"
#define FG_DIM       "\033[38;2;153;153;153m"
#define FG_SUBTLE    "\033[38;2;85;85;85m"
#define FG_BORDER    "\033[38;2;136;136;136m"
#define FG_PERM      "\033[38;2;177;185;249m"
#define FG_GREEN     "\033[38;2;78;186;101m"
#define FG_BLUE      "\033[38;2;115;165;245m"
#define FG_WARN      "\033[38;2;255;193;7m"
#define FG_DEL       "\033[38;2;255;176;186m"
#define FG_ADD       "\033[38;2;165;240;178m"
#define BG_DEFAULT   "\033[49m"
#define BG_DEL       "\033[48;2;88;32;40m"
#define BG_ADD       "\033[48;2;28;76;38m"

typedef struct {
    const char *fg;
    const char *bg;
    int bold;
    char text[256];
} Seg;

typedef struct {
    Seg segs[MAX_SEGS];
    int count;
    const char *fill_bg;
    char rule_utf8[8]; /* non-empty => repeat glyph cols times */
    const char *rule_fg;
} Line;

typedef struct {
    char data[65536];
    size_t len;
} OutBuf;

static volatile sig_atomic_t g_winch = 0;
static struct termios g_orig_termios;
static int g_raw_active = 0;

static void on_sigwinch(int sig) {
    (void)sig;
    g_winch = 1;
}

static void restore_term(void) {
    if (g_raw_active) {
        tcsetattr(STDIN_FILENO, TCSANOW, &g_orig_termios);
        g_raw_active = 0;
    }
    const char *show_cursor = "\033[?25h\033[?7h\033[0m";
    (void)!write(STDOUT_FILENO, show_cursor, strlen(show_cursor));
}

static uint64_t monotonic_ms(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (uint64_t)ts.tv_sec * 1000ULL + (uint64_t)ts.tv_nsec / 1000000ULL;
}

static int utf8_cp_len(unsigned char c) {
    if (c < 0x80) return 1;
    if ((c & 0xE0) == 0xC0) return 2;
    if ((c & 0xF0) == 0xE0) return 3;
    if ((c & 0xF8) == 0xF0) return 4;
    return 1;
}

static int utf8_cols(const char *s) {
    int n = 0;
    while (*s) {
        int step = utf8_cp_len((unsigned char)*s);
        for (int i = 0; i < step && *s; i++) s++;
        n++;
    }
    return n;
}

static void ob_append(OutBuf *b, const char *s, size_t n) {
    if (b->len + n >= sizeof(b->data)) {
        n = sizeof(b->data) - 1 - b->len;
    }
    if (n > 0) {
        memcpy(b->data + b->len, s, n);
        b->len += n;
        b->data[b->len] = '\0';
    }
}

static void ob_str(OutBuf *b, const char *s) {
    if (s) ob_append(b, s, strlen(s));
}

static void ob_fmt(OutBuf *b, const char *fmt, ...) {
    char tmp[512];
    va_list ap;
    va_start(ap, fmt);
    int n = vsnprintf(tmp, sizeof(tmp), fmt, ap);
    va_end(ap);
    if (n > 0) {
        size_t len = (size_t)n < sizeof(tmp) ? (size_t)n : sizeof(tmp) - 1;
        ob_append(b, tmp, len);
    }
}

static void write_all(int fd, const char *buf, size_t len) {
    size_t off = 0;
    while (off < len) {
        ssize_t w = write(fd, buf + off, len - off);
        if (w < 0) {
            if (errno == EINTR) continue;
            break;
        }
        off += (size_t)w;
    }
}

static void query_winsize(int *rows, int *cols) {
    struct winsize ws;
    memset(&ws, 0, sizeof(ws));
    if (ioctl(STDOUT_FILENO, TIOCGWINSZ, &ws) == 0 && ws.ws_row > 0 && ws.ws_col > 0) {
        *rows = (int)ws.ws_row;
        *cols = (int)ws.ws_col;
        return;
    }
    if (ioctl(STDIN_FILENO, TIOCGWINSZ, &ws) == 0 && ws.ws_row > 0 && ws.ws_col > 0) {
        *rows = (int)ws.ws_row;
        *cols = (int)ws.ws_col;
        return;
    }
    *rows = 40;
    *cols = 80;
}

static Line *push_line(Line *lines, int *count) {
    if (*count >= MAX_LINES) return NULL;
    Line *ln = &lines[(*count)++];
    memset(ln, 0, sizeof(*ln));
    return ln;
}

static void add_seg(Line *ln, const char *fg, const char *bg, int bold, const char *text) {
    if (!ln || ln->count >= MAX_SEGS || !text || !*text) return;
    Seg *sg = &ln->segs[ln->count++];
    sg->fg = fg ? fg : FG_WHITE;
    sg->bg = bg;
    sg->bold = bold;
    snprintf(sg->text, sizeof(sg->text), "%s", text);
}

static void add_blank(Line *lines, int *count) {
    (void)push_line(lines, count);
}

static void add_rule(Line *lines, int *count, const char *fg, const char *glyph) {
    Line *ln = push_line(lines, count);
    if (!ln) return;
    ln->rule_fg = fg;
    snprintf(ln->rule_utf8, sizeof(ln->rule_utf8), "%s", glyph);
}

static void add_simple(Line *lines, int *count, const char *fg, int bold, const char *text) {
    Line *ln = push_line(lines, count);
    if (!ln) return;
    add_seg(ln, fg, NULL, bold, text);
}

static void add_dot_line(Line *lines, int *count, const char *dot_fg, const char *rest_fg,
                         const char *text) {
    Line *ln = push_line(lines, count);
    if (!ln) return;
    add_seg(ln, dot_fg, NULL, 0, "\xe2\x97\x8f ");
    add_seg(ln, rest_fg, NULL, 0, text);
}

static void add_tool_call(Line *lines, int *count, const char *tool, const char *args) {
    Line *ln = push_line(lines, count);
    if (!ln) return;
    add_seg(ln, FG_BLUE, NULL, 0, "\xe2\x97\x8f ");
    add_seg(ln, FG_WHITE, NULL, 1, tool);
    add_seg(ln, FG_WHITE, NULL, 0, "(");
    add_seg(ln, FG_DIM, NULL, 0, args);
    add_seg(ln, FG_WHITE, NULL, 0, ")");
}

static void add_result_line(Line *lines, int *count, const char *prefix, const char *text) {
    Line *ln = push_line(lines, count);
    if (!ln) return;
    add_seg(ln, FG_DIM, NULL, 0, prefix);
    add_seg(ln, FG_DIM, NULL, 0, text);
}

/* Emit up to max_cols display cells of a Line, truncating safely at UTF-8 boundaries. */
static void render_line(OutBuf *ob, const Line *ln, int cols) {
    if (ln->rule_utf8[0] != '\0') {
        ob_str(ob, "\033[0m");
        if (ln->rule_fg) ob_str(ob, ln->rule_fg);
        for (int i = 0; i < cols; i++) ob_str(ob, ln->rule_utf8);
        ob_str(ob, "\033[0m\033[K");
        return;
    }

    int used = 0;
    const char *row_bg = ln->fill_bg;
    ob_str(ob, "\033[0m");
    if (row_bg) ob_str(ob, row_bg);

    for (int i = 0; i < ln->count && used < cols; i++) {
        const Seg *sg = &ln->segs[i];
        ob_str(ob, "\033[22m");
        if (sg->bold) ob_str(ob, "\033[1m");
        ob_str(ob, sg->fg ? sg->fg : FG_WHITE);
        if (sg->bg) {
            ob_str(ob, sg->bg);
        } else if (row_bg) {
            ob_str(ob, row_bg);
        } else {
            ob_str(ob, BG_DEFAULT);
        }

        const char *p = sg->text;
        while (*p && used < cols) {
            int step = utf8_cp_len((unsigned char)*p);
            size_t avail = strlen(p);
            if ((size_t)step > avail) step = (int)avail;
            ob_append(ob, p, (size_t)step);
            p += step;
            used++;
        }
    }

    if (row_bg) {
        for (; used < cols; used++) ob_append(ob, " ", 1);
        ob_str(ob, "\033[0m\033[K");
    } else {
        ob_str(ob, "\033[0m\033[K");
    }
}

static int build_confirmation_body(Line *lines, int cols, int draft_len) {
    int count = 0;
    Line *ln;

    /* Claude Code 2.1 header with Clawd mascot */
    ln = push_line(lines, &count);
    add_seg(ln, FG_CLAWD, NULL, 0, " \xe2\x96\x90\xe2\x96\x9b\xe2\x96\x88\xe2\x96\x88\xe2\x96\x88\xe2\x96\x9c\xe2\x96\x8c");
    add_seg(ln, FG_WHITE, NULL, 1, "   Claude Code");
    add_seg(ln, FG_DIM, NULL, 0, " v2.1.326");

    ln = push_line(lines, &count);
    add_seg(ln, FG_CLAWD, NULL, 0, "\xe2\x96\x9d\xe2\x96\x9c\xe2\x96\x88\xe2\x96\x88\xe2\x96\x88\xe2\x96\x88\xe2\x96\x88\xe2\x96\x9b\xe2\x96\x98");
    add_seg(ln, FG_DIM, NULL, 0, "  Opus 4.7 \xc2\xb7 Claude Max");

    ln = push_line(lines, &count);
    add_seg(ln, FG_CLAWD, NULL, 0, "  \xe2\x96\x98\xe2\x96\x98 \xe2\x96\x9d\xe2\x96\x9d");
    add_seg(ln, FG_DIM, NULL, 0, "    ~/src/payments-api");

    add_blank(lines, &count);

    /* User turn */
    ln = push_line(lines, &count);
    add_seg(ln, FG_WHITE, NULL, 1, "\xe2\x9d\xaf ");
    add_seg(ln, FG_WHITE, NULL, 0, "make the webhook retry back off and cap it");

    add_blank(lines, &count);

    /* Assistant intro */
    add_dot_line(lines, &count, FG_GREEN, FG_WHITE,
                 "I'll make retry back off and cap the wait.");

    add_blank(lines, &count);

    /* Tool steps (<= 49 cols each; keeps "     Wrote bench/latency.png" intact) */
    add_tool_call(lines, &count, "Search", "pattern: \"retry\", path: \"src\"");
    add_result_line(lines, &count, "  \xe2\x8e\xbf  ", "Found 4 files (ctrl+o to expand)");
    add_tool_call(lines, &count, "Read", "src/retry.js");
    add_result_line(lines, &count, "  \xe2\x8e\xbf  ", "Read 6 lines");
    add_tool_call(lines, &count, "Bash", "npm run bench -- --events 1000");
    add_result_line(lines, &count, "  \xe2\x8e\xbf  ", "p99 9.8s, 41 of 1000 events dropped");
    add_result_line(lines, &count, "     ", "Wrote bench/latency.png");

    add_blank(lines, &count);

    add_dot_line(lines, &count, FG_GREEN, FG_WHITE,
                 "It retries 3\xc3\x97 with no delay, dropping bursts.");
    add_tool_call(lines, &count, "Write", "src/backoff.js");
    add_result_line(lines, &count, "  \xe2\x8e\xbf  ", "Wrote 24 lines to src/backoff.js");
    add_tool_call(lines, &count, "Update", "src/retry.js");

    if (draft_len > 0) {
        /* 2.1.326 draftHiddenCue collapses the inline dialog while typing in the composer */
        add_blank(lines, &count);
        add_simple(lines, &count, FG_DIM, 0,
                   "  Claude has a question for you \xe2\x80\x94 it shows once");
        add_simple(lines, &count, FG_DIM, 0,
                   "  you send or clear what you're typing.");
        return count;
    }

    /* Inline Edit file permission dialog */
    add_rule(lines, &count, FG_PERM, "\xe2\x94\x80");
    add_simple(lines, &count, FG_PERM, 1, " Edit file");
    add_rule(lines, &count, FG_SUBTLE, "\xe2\x95\x8c");
    add_simple(lines, &count, FG_WHITE, 1, " src/retry.js");
    add_rule(lines, &count, FG_SUBTLE, "\xe2\x95\x8c");

    ln = push_line(lines, &count);
    add_seg(ln, FG_DIM, NULL, 0, "   1  ");
    add_seg(ln, FG_WHITE, NULL, 0, "function retry(fn, times = 3) {");

    ln = push_line(lines, &count);
    add_seg(ln, FG_DIM, NULL, 0, "   2    ");
    add_seg(ln, FG_WHITE, NULL, 0, "for (let i = 0; i < times; i++) {");

    ln = push_line(lines, &count);
    ln->fill_bg = BG_DEL;
    add_seg(ln, FG_DIM, BG_DEL, 0, "   3 ");
    add_seg(ln, FG_DEL, BG_DEL, 0, "-    return fn();");

    ln = push_line(lines, &count);
    ln->fill_bg = BG_ADD;
    add_seg(ln, FG_DIM, BG_ADD, 0, "   3 ");
    add_seg(ln, FG_ADD, BG_ADD, 0, "+    return backoff(fn, times, 30_000);");

    ln = push_line(lines, &count);
    add_seg(ln, FG_DIM, NULL, 0, "   4    ");
    add_seg(ln, FG_WHITE, NULL, 0, "}");

    add_rule(lines, &count, FG_SUBTLE, "\xe2\x95\x8c");

    ln = push_line(lines, &count);
    add_seg(ln, FG_WHITE, NULL, 0, " Do you want to make this edit to ");
    add_seg(ln, FG_WHITE, NULL, 1, "retry.js");
    add_seg(ln, FG_WHITE, NULL, 0, "?");

    ln = push_line(lines, &count);
    add_seg(ln, FG_PERM, NULL, 1, " \xe2\x9d\xaf 1. Yes");

    if (cols >= 56) {
        add_simple(lines, &count, FG_WHITE, 0,
                   "   2. Yes, allow all edits during this session (shift+tab)");
    } else {
        add_simple(lines, &count, FG_WHITE, 0,
                   "   2. Yes, allow all edits during this session");
        add_simple(lines, &count, FG_WHITE, 0,
                   "      (shift+tab)");
    }
    add_simple(lines, &count, FG_WHITE, 0, "   3. No");
    add_simple(lines, &count, FG_DIM, 0, " Esc to cancel \xc2\xb7 Tab to amend");

    return count;
}

static void emit_wrapped_text(Line *lines, int *count, int cols,
                              const char *first_fg, int first_bold,
                              const char *prefix_fg, const char *prefix,
                              const char *cont_indent, const char *text) {
    int plen = utf8_cols(prefix);
    int ilen = utf8_cols(cont_indent);
    int first = 1;
    const char *p = text;

    while (*p && *count < MAX_LINES) {
        int lead_cols = first ? plen : ilen;
        int room = cols - lead_cols;
        if (room < 8) room = 8;

        const char *scan = p;
        const char *last_space = NULL;
        int cur = 0;
        while (*scan && cur < room) {
            if (*scan == ' ') last_space = scan;
            int step = utf8_cp_len((unsigned char)*scan);
            for (int k = 0; k < step && *scan; k++) scan++;
            cur++;
        }

        const char *cut = (*scan == '\0') ? scan : (last_space && last_space > p ? last_space : scan);
        Line *ln = push_line(lines, count);
        if (!ln) break;

        if (first && prefix[0] != '\0') {
            add_seg(ln, prefix_fg ? prefix_fg : first_fg, NULL, first_bold, prefix);
        } else if (!first && cont_indent[0] != '\0') {
            add_seg(ln, first_fg, NULL, 0, cont_indent);
        }

        size_t chunk_len = (size_t)(cut - p);
        if (chunk_len >= sizeof(ln->segs[0].text)) chunk_len = sizeof(ln->segs[0].text) - 1;
        if (chunk_len > 0) {
            char tmp[256];
            memcpy(tmp, p, chunk_len);
            tmp[chunk_len] = '\0';
            add_seg(ln, first_fg, NULL, first_bold && first && prefix[0] == '\0', tmp);
        }

        p = cut;
        while (*p == ' ') p++;
        first = 0;
        if (!*p) break;
    }
}

static int build_working_body(Line *lines, const char *path, int cols) {
    FILE *f = fopen(path, "r");
    int count = 0;
    if (!f) {
        add_simple(lines, &count, FG_DIM, 0, "  Claude Code");
        return count;
    }

    char raw[1024];
    while (fgets(raw, sizeof(raw), f) && count < MAX_LINES) {
        size_t len = strlen(raw);
        while (len > 0 && (raw[len - 1] == '\n' || raw[len - 1] == '\r')) {
            raw[--len] = '\0';
        }
        if (len == 0) {
            add_blank(lines, &count);
        } else if (strncmp(raw, "> ", 2) == 0) {
            emit_wrapped_text(lines, &count, cols, FG_WHITE, 0, FG_WHITE, "\xe2\x9d\xaf ", "  ",
                              raw + 2);
        } else if (strncmp(raw, "\xe2\x97\x8f ", 4) == 0) {
            const char *body = raw + 4;
            const char *paren = strchr(body, '(');
            if (paren && paren > body && strchr(paren, ')') && utf8_cols(raw) <= cols) {
                char tool[64];
                size_t tlen = (size_t)(paren - body);
                if (tlen > 0 && tlen < sizeof(tool) && !memchr(body, ' ', tlen)) {
                    memcpy(tool, body, tlen);
                    tool[tlen] = '\0';
                    Line *ln = push_line(lines, &count);
                    if (ln) {
                        add_seg(ln, FG_BLUE, NULL, 0, "\xe2\x97\x8f ");
                        add_seg(ln, FG_WHITE, NULL, 1, tool);
                        add_seg(ln, FG_DIM, NULL, 0, paren);
                    }
                    continue;
                }
            }
            emit_wrapped_text(lines, &count, cols, FG_WHITE, 0, FG_GREEN, "\xe2\x97\x8f ", "  ",
                              body);
        } else if (strncmp(raw, "  \xe2\x8e\xbf", 5) == 0 || strncmp(raw, "     ", 5) == 0) {
            emit_wrapped_text(lines, &count, cols, FG_DIM, 0, FG_DIM, "", "     ", raw);
        } else {
            emit_wrapped_text(lines, &count, cols, FG_WHITE, 0, FG_WHITE, "", "  ", raw);
        }
    }
    fclose(f);
    return count;
}

static void draw_screen(const char *working_path, const char *draft, int draft_len) {
    int rows = 40, cols = 80;
    query_winsize(&rows, &cols);
    if (rows < 6) rows = 6;
    if (cols < 12) cols = 12;

    Line lines[MAX_LINES];
    int total = working_path
                    ? build_working_body(lines, working_path, cols)
                    : build_confirmation_body(lines, cols, draft_len);

    int composer_rows = (rows >= 8) ? 4 : 3;
    int avail = rows - composer_rows;
    int start = (total > avail) ? (total - avail) : 0;

    OutBuf ob;
    ob.len = 0;
    ob.data[0] = '\0';

    /* Begin synchronized update, hide cursor, reset style, home */
    ob_str(&ob, "\033[?2026h\033[?25l\033[?7h\033[0m\033[H");

    for (int r = 0; r < avail; r++) {
        ob_fmt(&ob, "\033[%d;1H", r + 1);
        if (start + r < total) {
            render_line(&ob, &lines[start + r], cols);
        } else {
            ob_str(&ob, "\033[0m\033[K");
        }
    }

    int base = avail + 1;

    /* Composer top rule */
    ob_fmt(&ob, "\033[%d;1H\033[0m%s", base, FG_BORDER);
    for (int c = 0; c < cols; c++) ob_str(&ob, "\xe2\x94\x80");
    ob_str(&ob, "\033[0m\033[K");

    /* Composer input row */
    ob_fmt(&ob, "\033[%d;1H\033[0m\033[1m%s\xe2\x9d\xaf \033[22m", base + 1, FG_WHITE);
    if (draft_len > 0) {
        int max_vis = cols - 4;
        if (max_vis < 1) max_vis = 1;
        const char *shown = draft;
        if (draft_len > max_vis) {
            shown = draft + (draft_len - max_vis);
        }
        ob_append(&ob, shown, strlen(shown));
        ob_str(&ob, "\033[7m \033[0m");
    }
    ob_str(&ob, "\033[0m\033[K");

    /* Composer bottom rule */
    ob_fmt(&ob, "\033[%d;1H\033[0m%s", base + 2, FG_BORDER);
    for (int c = 0; c < cols; c++) ob_str(&ob, "\xe2\x94\x80");
    ob_str(&ob, "\033[0m\033[K");

    /* Bottom status bar matching current Claude Code auto-mode footer */
    if (composer_rows == 4) {
        /* Disable DECAWM on the bottom-most row so nothing scrolls the viewport */
        ob_fmt(&ob, "\033[?7l\033[%d;1H\033[0m", base + 3);
        if (cols >= 50) {
            ob_fmt(&ob, "  %s\xe2\x8f\xb5\xe2\x8f\xb5 auto mode on%s (shift+tab to cycle) \xc2\xb7 \xe2\x86\x90 1 agent",
                   FG_WARN, FG_DIM);
        } else if (cols >= 38) {
            ob_fmt(&ob, "  %s\xe2\x8f\xb5\xe2\x8f\xb5 auto mode on%s (shift+tab to cycle)",
                   FG_WARN, FG_DIM);
        } else if (cols >= 18) {
            ob_fmt(&ob, "  %s\xe2\x8f\xb5\xe2\x8f\xb5 auto mode on", FG_WARN);
        } else {
            ob_fmt(&ob, "  %s\xe2\x8f\xb5\xe2\x8f\xb5 auto", FG_WARN);
        }
        ob_str(&ob, "\033[0m\033[K\033[?7h");
    }

    /* Park hardware cursor inside the composer row (kept hidden via DECTCEM) */
    int cursor_col = 3 + (draft_len > 0 ? (draft_len < cols - 4 ? draft_len : cols - 4) : 0);
    ob_fmt(&ob, "\033[%d;%dH\033[?2026l", base + 1, cursor_col);

    write_all(STDOUT_FILENO, ob.data, ob.len);
}

static void drain_escape_seq(void) {
    struct pollfd pfd = {.fd = STDIN_FILENO, .events = POLLIN};
    char junk[32];
    while (poll(&pfd, 1, 15) > 0 && (pfd.revents & POLLIN)) {
        ssize_t n = read(STDIN_FILENO, junk, sizeof(junk));
        if (n <= 0) break;
    }
}

int main(int argc, char **argv) {
    const char *working_path = NULL;
    if (argc == 3 && strcmp(argv[1], "--working") == 0) {
        working_path = argv[2];
    } else if (argc != 1) {
        fprintf(stderr, "usage: %s [--working <text-file>]\n", argv[0]);
        return 64;
    }

    struct sigaction sa;
    memset(&sa, 0, sizeof(sa));
    sa.sa_handler = on_sigwinch;
    sigemptyset(&sa.sa_mask);
    sigaction(SIGWINCH, &sa, NULL);

    if (isatty(STDIN_FILENO)) {
        if (tcgetattr(STDIN_FILENO, &g_orig_termios) == 0) {
            struct termios raw = g_orig_termios;
            raw.c_iflag &= (tcflag_t) ~(ICRNL | IXON);
            raw.c_lflag &= (tcflag_t) ~(ICANON | ECHO | ISIG | IEXTEN);
            raw.c_cc[VMIN] = 1;
            raw.c_cc[VTIME] = 0;
            if (tcsetattr(STDIN_FILENO, TCSANOW, &raw) == 0) {
                g_raw_active = 1;
                atexit(restore_term);
            }
        }
    }

    char draft[MAX_DRAFT + 1];
    int draft_len = 0;
    draft[0] = '\0';
    uint64_t last_input_ms = 0;

    /* Clear screen + scrollback on startup, then draw full frame */
    const char *clear_scrollback = "\033[H\033[2J\033[3J";
    write_all(STDOUT_FILENO, clear_scrollback, strlen(clear_scrollback));
    draw_screen(working_path, draft, draft_len);

    int stdin_eof = 0;
    for (;;) {
        if (g_winch) {
            g_winch = 0;
            draw_screen(working_path, draft, draft_len);
        }

        if (stdin_eof) {
            pause();
            continue;
        }

        struct pollfd pfd = {.fd = STDIN_FILENO, .events = POLLIN};
        int timeout_ms = (draft_len > 0) ? 500 : -1;
        int rc = poll(&pfd, 1, timeout_ms);
        if (rc < 0) {
            if (errno == EINTR) continue;
            break;
        }

        if (draft_len > 0 && monotonic_ms() - last_input_ms >= IDLE_RESET_MS) {
            draft_len = 0;
            draft[0] = '\0';
            draw_screen(working_path, draft, draft_len);
        }

        if (rc == 0 || !(pfd.revents & POLLIN)) {
            if (pfd.revents & (POLLHUP | POLLERR | POLLNVAL)) {
                stdin_eof = 1;
            }
            continue;
        }

        unsigned char buf[128];
        ssize_t n = read(STDIN_FILENO, buf, sizeof(buf));
        if (n <= 0) {
            if (n < 0 && errno == EINTR) continue;
            stdin_eof = 1;
            continue;
        }

        if (working_path != NULL) {
            /* Working panes stay untouched until killed; redraw on resize only. */
            continue;
        }

        int dirty = 0;
        for (ssize_t i = 0; i < n; i++) {
            unsigned char ch = buf[i];
            if (ch == 0x1b) {
                /* Skip any remaining bytes in the same CSI burst and reset draft. */
                drain_escape_seq();
                if (draft_len != 0) {
                    draft_len = 0;
                    draft[0] = '\0';
                    dirty = 1;
                }
                break;
            } else if (ch == 0x15 || ch == 0x03 || ch == '\r' || ch == '\n') {
                if (draft_len != 0) {
                    draft_len = 0;
                    draft[0] = '\0';
                    dirty = 1;
                }
            } else if (ch == 0x7f || ch == 0x08) {
                if (draft_len > 0) {
                    draft[--draft_len] = '\0';
                    last_input_ms = monotonic_ms();
                    dirty = 1;
                }
            } else if (isprint(ch)) {
                if (draft_len < MAX_DRAFT) {
                    draft[draft_len++] = (char)ch;
                    draft[draft_len] = '\0';
                    last_input_ms = monotonic_ms();
                    dirty = 1;
                }
            }
        }
        if (dirty) {
            draw_screen(working_path, draft, draft_len);
        }
    }

    return 0;
}
