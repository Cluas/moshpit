#!/usr/bin/env python3
"""A local fake model server for the CLI lab: OpenAI chat completions,
OpenAI Responses, Anthropic Messages and Gemini streamGenerateContent,
each streaming one canned multi-line reply. Never forwards anywhere.

    fake_model.py <port> [<log>]
"""
import json
import os
import sys
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

REPLY = [
    "Sure. Here is a short plan:\n",
    "1. Read the input box code.\n",
    "2. Repaint only what changed.\n",
    "3. Keep the cursor after the text.\n",
    "Done: 你好，世界。",
]
# LAB_LONG=N: a reply of N more lines, one every LAB_PACE s (a long answer
# streaming in, with the CLI's spinner and status line ticking meanwhile).
REPLY += [f"Line {i} of a longer answer, streamed in slowly.\n" for i in range(int(os.environ.get("LAB_LONG", 0)))]
PACE = float(os.environ.get("LAB_PACE", "0.15"))
MODEL = "lab-model"
LOG = open(sys.argv[2], "a") if len(sys.argv) > 2 else None


def log(*parts):
    if LOG:
        LOG.write(" ".join(str(p) for p in parts) + "\n")
        LOG.flush()


class H(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *a):
        pass

    def body(self):
        n = int(self.headers.get("content-length") or 0)
        raw = self.rfile.read(n) if n else b""
        try:
            return json.loads(raw or b"{}")
        except Exception:
            return {}

    def send_json(self, obj, code=200):
        data = json.dumps(obj).encode()
        self.send_response(code)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def sse_start(self):
        self.send_response(200)
        self.send_header("content-type", "text/event-stream")
        self.send_header("cache-control", "no-cache")
        self.send_header("connection", "close")
        self.end_headers()
        self.close_connection = True

    def sse(self, data, event=None):
        out = ""
        if event:
            out += f"event: {event}\n"
        out += "data: " + (data if isinstance(data, str) else json.dumps(data)) + "\n\n"
        self.wfile.write(out.encode())
        self.wfile.flush()

    def do_GET(self):
        log("GET", self.path)
        if "models" in self.path:
            self.send_json({"object": "list", "data": [{"id": MODEL, "object": "model", "owned_by": "lab"}],
                            "models": [{"name": "models/" + MODEL}]})
        else:
            self.send_json({})

    def do_HEAD(self):
        self.send_response(200)
        self.send_header("content-length", "0")
        self.end_headers()

    def do_POST(self):
        req = self.body()
        log("POST", self.path, "stream=%s" % req.get("stream"))
        p = self.path
        if p.rstrip("/").endswith("/messages") or "/messages?" in p:
            return self.anthropic(req)
        if "/responses" in p:
            return self.openai_responses(req)
        if "generatecontent" in p.lower():
            return self.gemini(p)
        if "chat/completions" in p:
            return self.chat(req)
        self.send_json({})

    def chat(self, req):
        if not req.get("stream"):
            return self.send_json({"id": "c1", "object": "chat.completion", "created": 0, "model": MODEL,
                                   "choices": [{"index": 0, "message": {"role": "assistant", "content": "".join(REPLY)},
                                                "finish_reason": "stop"}],
                                   "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}})
        self.sse_start()
        base = {"id": "c1", "object": "chat.completion.chunk", "created": 0, "model": MODEL}
        self.sse({**base, "choices": [{"index": 0, "delta": {"role": "assistant", "content": ""}, "finish_reason": None}]})
        for part in REPLY:
            time.sleep(PACE)
            self.sse({**base, "choices": [{"index": 0, "delta": {"content": part}, "finish_reason": None}]})
        self.sse({**base, "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}],
                  "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}})
        self.sse("[DONE]")

    def anthropic(self, req):
        msg = {"id": "msg_1", "type": "message", "role": "assistant", "model": req.get("model", MODEL),
               "content": [], "stop_reason": None, "stop_sequence": None,
               "usage": {"input_tokens": 1, "output_tokens": 1}}
        if not req.get("stream"):
            msg["content"] = [{"type": "text", "text": "".join(REPLY)}]
            msg["stop_reason"] = "end_turn"
            return self.send_json(msg)
        self.sse_start()
        self.sse({"type": "message_start", "message": msg}, "message_start")
        self.sse({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}},
                 "content_block_start")
        for part in REPLY:
            time.sleep(PACE)
            self.sse({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": part}},
                     "content_block_delta")
        self.sse({"type": "content_block_stop", "index": 0}, "content_block_stop")
        self.sse({"type": "message_delta", "delta": {"stop_reason": "end_turn", "stop_sequence": None},
                  "usage": {"output_tokens": 20}}, "message_delta")
        self.sse({"type": "message_stop"}, "message_stop")

    def openai_responses(self, req):
        self.sse_start()
        rid = "resp_1"
        resp = {"id": rid, "object": "response", "created_at": 0, "status": "in_progress", "model": MODEL,
                "output": []}
        self.sse({"type": "response.created", "response": resp}, "response.created")
        item = {"id": "msg_1", "type": "message", "role": "assistant", "status": "in_progress", "content": []}
        self.sse({"type": "response.output_item.added", "output_index": 0, "item": item},
                 "response.output_item.added")
        self.sse({"type": "response.content_part.added", "item_id": "msg_1", "output_index": 0, "content_index": 0,
                  "part": {"type": "output_text", "text": "", "annotations": []}}, "response.content_part.added")
        for part in REPLY:
            time.sleep(PACE)
            self.sse({"type": "response.output_text.delta", "item_id": "msg_1", "output_index": 0,
                      "content_index": 0, "delta": part}, "response.output_text.delta")
        text = "".join(REPLY)
        self.sse({"type": "response.output_text.done", "item_id": "msg_1", "output_index": 0, "content_index": 0,
                  "text": text}, "response.output_text.done")
        done_item = {**item, "status": "completed",
                     "content": [{"type": "output_text", "text": text, "annotations": []}]}
        self.sse({"type": "response.output_item.done", "output_index": 0, "item": done_item},
                 "response.output_item.done")
        self.sse({"type": "response.completed",
                  "response": {**resp, "status": "completed", "output": [done_item],
                               "usage": {"input_tokens": 1, "output_tokens": 1, "total_tokens": 2,
                                         "input_tokens_details": {"cached_tokens": 0},
                                         "output_tokens_details": {"reasoning_tokens": 0}}}},
                 "response.completed")

    def gemini(self, path):
        sse = "alt=sse" in path
        chunks = [{"candidates": [{"content": {"role": "model", "parts": [{"text": part}]}, "index": 0}]}
                  for part in REPLY]
        chunks[-1]["candidates"][0]["finishReason"] = "STOP"
        chunks[-1]["usageMetadata"] = {"promptTokenCount": 1, "candidatesTokenCount": 1, "totalTokenCount": 2}
        if not sse:
            return self.send_json(chunks if "stream" in path.lower() else chunks[-1])
        self.sse_start()
        for c in chunks:
            time.sleep(PACE)
            self.sse(c)


if __name__ == "__main__":
    ThreadingHTTPServer(("127.0.0.1", int(sys.argv[1])), H).serve_forever()
