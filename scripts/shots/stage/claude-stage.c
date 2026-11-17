/*
 * Claude Code 2.1.326 terminal stage program for screenshot and UI test fixtures.
 *
 * Replays either:
 *   claude                   Edit confirmation dialog + persistent bottom composer
 *   claude --working <file>  Active turn log from <file> + persistent bottom composer
 *
 * Queries the PTY's actual rows and columns (TIOCGWINSZ) on startup and on every
 * SIGWINCH, dynamically reflowing prompts, assistant prose, tool calls, tool
 * results, inline diffs, and the confirmation/draft-hidden footer to fill the
 * terminal width while keeping the 4-row composer docked to the bottom edge.
 * Pending composer drafts reset cleanly on Esc, Ctrl-U, Ctrl-C, Enter, or a 10s
 * idle timeout.
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
#define SEG_TEXT_CAP 512
#define MAX_DRAFT 255
#define IDLE_RESET_MS 10000

/* 24-bit TrueColor ANSI SGR foreground/background codes matching dark theme */
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
    char text[SEG_TEXT_CAP];
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

static void add_result_line(Line *lines, int *count, const char *prefix, const char *text) {
    Line *ln = push_line(lines, count);
    if (!ln) return;
    add_seg(ln, FG_DIM, NULL, 0, prefix);
    add_seg(ln, FG_DIM, NULL, 0, text);
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

        const char *cut = (*scan == '\0' || *scan == ' ')
                              ? scan
                              : (last_space && last_space > p ? last_space : scan);
        Line *ln = push_line(lines, count);
        if (!ln) break;

        if (first && prefix[0] != '\0') {
            add_seg(ln, prefix_fg ? prefix_fg : first_fg, NULL, first_bold, prefix);
        } else if (!first && cont_indent[0] != '\0') {
            add_seg(ln, first_fg, NULL, 0, cont_indent);
        }

        size_t chunk_len = (size_t)(cut - p);
        if (chunk_len >= SEG_TEXT_CAP) chunk_len = SEG_TEXT_CAP - 1;
        if (chunk_len > 0) {
            char tmp[SEG_TEXT_CAP];
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

static void emit_tool_call(Line *lines, int *count, int cols,
                           const char *tool, const char *args) {
    int head_cols = 2 + utf8_cols(tool) + 1; /* "● " + tool + "(" */
    int args_cols = utf8_cols(args);
    if (head_cols + args_cols + 1 <= cols) {
        Line *ln = push_line(lines, count);
        if (!ln) return;
        add_seg(ln, FG_BLUE, NULL, 0, "\xe2\x97\x8f ");
        add_seg(ln, FG_WHITE, NULL, 1, tool);
        add_seg(ln, FG_WHITE, NULL, 0, "(");
        add_seg(ln, FG_DIM, NULL, 0, args);
        add_seg(ln, FG_WHITE, NULL, 0, ")");
        return;
    }

    const char *cont = "     ";
    int cont_cols = 5;
    int first = 1;
    const char *p = args;
    while (*p && *count < MAX_LINES) {
        int lead = first ? head_cols : cont_cols;
        int rem_cols = utf8_cols(p);
        /* Reserve 1 col for closing ')' when the rest fits on this line */
        if (lead + rem_cols + 1 <= cols) {
            Line *ln = push_line(lines, count);
            if (!ln) break;
            if (first) {
                add_seg(ln, FG_BLUE, NULL, 0, "\xe2\x97\x8f ");
                add_seg(ln, FG_WHITE, NULL, 1, tool);
                add_seg(ln, FG_WHITE, NULL, 0, "(");
            } else {
                add_seg(ln, FG_DIM, NULL, 0, cont);
            }
            add_seg(ln, FG_DIM, NULL, 0, p);
            add_seg(ln, FG_WHITE, NULL, 0, ")");
            break;
        }

        int room = cols - lead;
        if (room < 6) room = 6;
        const char *scan = p;
        const char *last_break = NULL;
        int cur = 0;
        while (*scan && cur < room) {
            if (*scan == ' ') last_break = scan;
            int step = utf8_cp_len((unsigned char)*scan);
            for (int k = 0; k < step && *scan; k++) scan++;
            cur++;
        }
        const char *cut = (*scan == '\0' || *scan == ' ')
                              ? scan
                              : (last_break && last_break > p ? last_break : scan);
        Line *ln = push_line(lines, count);
        if (!ln) break;
        if (first) {
            add_seg(ln, FG_BLUE, NULL, 0, "\xe2\x97\x8f ");
            add_seg(ln, FG_WHITE, NULL, 1, tool);
            add_seg(ln, FG_WHITE, NULL, 0, "(");
        } else {
            add_seg(ln, FG_DIM, NULL, 0, cont);
        }
        size_t chunk_len = (size_t)(cut - p);
        if (chunk_len >= SEG_TEXT_CAP) chunk_len = SEG_TEXT_CAP - 1;
        if (chunk_len > 0) {
            char tmp[SEG_TEXT_CAP];
            memcpy(tmp, p, chunk_len);
            tmp[chunk_len] = '\0';
            add_seg(ln, FG_DIM, NULL, 0, tmp);
        }
        p = cut;
        while (*p == ' ') p++;
        first = 0;
        if (!*p) {
            add_seg(ln, FG_WHITE, NULL, 0, ")");
            break;
        }
    }
}

static void emit_code_line(Line *lines, int *count, int cols,
                           const char *fill_bg, const char *gutter_fg,
                           const char *gutter, const char *marker,
                           const char *cont_gutter,
                           const char *code_fg, const char *code) {
    int gcols = utf8_cols(gutter) + utf8_cols(marker);
    int ccols = utf8_cols(cont_gutter);
    int first = 1;
    const char *p = code;

    if (!*p) {
        Line *ln = push_line(lines, count);
        if (!ln) return;
        ln->fill_bg = fill_bg;
        add_seg(ln, gutter_fg, fill_bg, 0, gutter);
        if (marker && *marker) add_seg(ln, code_fg, fill_bg, 0, marker);
        return;
    }

    while (*p && *count < MAX_LINES) {
        int lead = first ? gcols : ccols;
        int room = cols - lead;
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

        const char *cut = (*scan == '\0' || *scan == ' ')
                              ? scan
                              : (last_space && last_space > p ? last_space : scan);
        Line *ln = push_line(lines, count);
        if (!ln) break;
        ln->fill_bg = fill_bg;
        if (first) {
            add_seg(ln, gutter_fg, fill_bg, 0, gutter);
            if (marker && *marker) add_seg(ln, code_fg, fill_bg, 0, marker);
        } else {
            add_seg(ln, gutter_fg, fill_bg, 0, cont_gutter);
        }

        size_t chunk_len = (size_t)(cut - p);
        if (chunk_len >= SEG_TEXT_CAP) chunk_len = SEG_TEXT_CAP - 1;
        if (chunk_len > 0) {
            char tmp[SEG_TEXT_CAP];
            memcpy(tmp, p, chunk_len);
            tmp[chunk_len] = '\0';
            add_seg(ln, code_fg, fill_bg, 0, tmp);
        }

        p = cut;
        while (*p == ' ') p++;
        first = 0;
        if (!*p) break;
    }
}

/* Emit up to cols display cells of a Line, truncating safely at UTF-8 boundaries. */
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

    /* User turn: dynamically wrapped across the terminal's actual columns */
    emit_wrapped_text(lines, &count, cols, FG_WHITE, 0, FG_WHITE, "\xe2\x9d\xaf ", "  ",
                      "webhook retries hammer 429 endpoints and drop bursts \xe2\x80\x94 "
                      "add full-jitter backoff and cap the wait at 30s");

    add_blank(lines, &count);

    /* Assistant intro */
    emit_wrapped_text(lines, &count, cols, FG_WHITE, 0, FG_GREEN, "\xe2\x97\x8f ", "  ",
                      "I'll benchmark burst drops in src/retry.js, then add "
                      "full-jitter exponential backoff capped at 30s.");

    add_blank(lines, &count);

    /* Tool steps (keeps "     Wrote bench/latency.png" on its own ASCII row for B817/test06) */
    emit_tool_call(lines, &count, cols, "Search", "pattern: \"retry|backoff\", path: \"src\"");
    emit_wrapped_text(lines, &count, cols, FG_DIM, 0, FG_DIM, "  \xe2\x8e\xbf  ", "     ",
                      "Found 4 files (src/retry.js, src/webhook.js, src/queue.js, test/retry.test.js)");
    emit_tool_call(lines, &count, cols, "Bash", "npm run bench -- --events 1000 --burst 32");
    emit_wrapped_text(lines, &count, cols, FG_DIM, 0, FG_DIM, "  \xe2\x8e\xbf  ", "     ",
                      "p99 9.8s \xc2\xb7 41 of 1000 events dropped on HTTP 429 bursts");
    add_result_line(lines, &count, "     ", "Wrote bench/latency.png");

    add_blank(lines, &count);

    emit_wrapped_text(lines, &count, cols, FG_WHITE, 0, FG_GREEN, "\xe2\x97\x8f ", "  ",
                      "src/retry.js fires all three attempts with zero delay, burning the "
                      "retry budget inside one second on a 429 burst. I'll cap full-jitter "
                      "exponential backoff at 30s.");
    emit_tool_call(lines, &count, cols, "Write", "src/backoff.js");
    emit_wrapped_text(lines, &count, cols, FG_DIM, 0, FG_DIM, "  \xe2\x8e\xbf  ", "     ",
                      "Wrote 24 lines to src/backoff.js (500ms base, 30_000ms cap, full jitter)");
    emit_tool_call(lines, &count, cols, "Update", "src/retry.js");

    if (draft_len > 0) {
        /* 2.1.326 draftHiddenCue collapses the inline dialog while typing in the composer */
        add_blank(lines, &count);
        emit_wrapped_text(lines, &count, cols, FG_DIM, 0, FG_DIM, "  ", "  ",
                          "Claude has a question for you \xe2\x80\x94 it shows once "
                          "you send or clear what you're typing.");
        return count;
    }

    /* Inline Edit file permission dialog */
    add_rule(lines, &count, FG_PERM, "\xe2\x94\x80");
    add_simple(lines, &count, FG_PERM, 1, " Edit file");
    add_rule(lines, &count, FG_SUBTLE, "\xe2\x95\x8c");
    add_simple(lines, &count, FG_WHITE, 1, " src/retry.js");
    add_rule(lines, &count, FG_SUBTLE, "\xe2\x95\x8c");

    emit_code_line(lines, &count, cols, NULL, FG_DIM, "   1  ", "", "        ",
                   FG_WHITE, "export async function retry(fn, times = 3) {");
    emit_code_line(lines, &count, cols, NULL, FG_DIM, "   2    ", "", "          ",
                   FG_WHITE, "for (let i = 0; i < times; i++) {");
    emit_code_line(lines, &count, cols, BG_DEL, FG_DIM, "   3 ", "-    ", "          ",
                   FG_DEL, "return await fn();");
    emit_code_line(lines, &count, cols, BG_ADD, FG_DIM, "   3 ", "+    ", "          ",
                   FG_ADD, "return await backoff(fn, i, { baseMs: 500, capMs: 30_000 });");
    emit_code_line(lines, &count, cols, NULL, FG_DIM, "   4    ", "", "          ",
                   FG_WHITE, "}");

    add_rule(lines, &count, FG_SUBTLE, "\xe2\x95\x8c");

    ln = push_line(lines, &count);
    add_seg(ln, FG_WHITE, NULL, 0, " Do you want to make this edit to ");
    add_seg(ln, FG_WHITE, NULL, 1, "retry.js");
    add_seg(ln, FG_WHITE, NULL, 0, "?");

    ln = push_line(lines, &count);
    add_seg(ln, FG_PERM, NULL, 1, " \xe2\x9d\xaf 1. Yes");

    emit_wrapped_text(lines, &count, cols, FG_WHITE, 0, FG_WHITE, "   ", "      ",
                      "2. Yes, allow all edits during this session (shift+tab)");
    add_simple(lines, &count, FG_WHITE, 0, "   3. No");
    add_simple(lines, &count, FG_DIM, 0, " Esc to cancel \xc2\xb7 Tab to amend");

    return count;
}

static int build_working_body(Line *lines, const char *path, int cols) {
    FILE *f = fopen(path, "r");
    int count = 0;
    if (!f) {
        add_simple(lines, &count, FG_DIM, 0, "  Claude Code");
        return count;
    }

    char raw[2048];
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
            const char *close = paren ? strrchr(paren, ')') : NULL;
            if (paren && paren > body && close && close[1] == '\0') {
                char tool[64];
                size_t tlen = (size_t)(paren - body);
                if (tlen > 0 && tlen < sizeof(tool) && !memchr(body, ' ', tlen)) {
                    memcpy(tool, body, tlen);
                    tool[tlen] = '\0';
                    char args[1024];
                    size_t alen = (size_t)(close - (paren + 1));
                    if (alen >= sizeof(args)) alen = sizeof(args) - 1;
                    memcpy(args, paren + 1, alen);
                    args[alen] = '\0';
                    emit_tool_call(lines, &count, cols, tool, args);
                    continue;
                }
            }
            emit_wrapped_text(lines, &count, cols, FG_WHITE, 0, FG_GREEN, "\xe2\x97\x8f ", "  ",
                              body);
        } else if (strncmp(raw, "  \xe2\x8e\xbf  ", 7) == 0) {
            emit_wrapped_text(lines, &count, cols, FG_DIM, 0, FG_DIM, "  \xe2\x8e\xbf  ", "     ",
                              raw + 7);
        } else if (strncmp(raw, "     ", 5) == 0) {
            emit_wrapped_text(lines, &count, cols, FG_DIM, 0, FG_DIM, "     ", "     ",
                              raw + 5);
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
