# Python port of DictationTranscript.collapsingRepeats (B670).
def collapse(text, minimum=6):
    c = list(text); out = []; i = 0
    while i < len(c):
        done = False
        for L in range(2, 17):
            if i + L * minimum > len(c): break
            unit = c[i:i+L]; end = i + L; n = 1
            while end + L <= len(c) and c[end:end+L] == unit:
                n += 1; end += L
            if n < minimum: continue
            out += unit; i = end; done = True; break
        if not done:
            out.append(c[i]); i += 1
    return ''.join(out)
