// Interop harness for the push relay's Go side: the relay's UNCHANGED
// push-relay/sealbox (imported through a replace directive) and
// push-relay/token.go (copied next to this file by harness/interop.py, which
// writes the go.mod). Not part of the relay.
//
// One command per stdin line, one result line each:
//
//	seal <secret> <iv|-> <plaintext hex>        {"v":1,"iv":…,"ct":…,"mac":…}
//	open <secret> <envelope json hex>           ok <plaintext hex> | err <message>
//	mint <master hex> <apns token> <conn> <iat> <send token>
//	verify <master hex> <token> <apns token> <conn> <iat> <now> <ttl seconds>
package main

import (
	"bufio"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"os"
	"strconv"
	"strings"
	"time"

	"github.com/cluas/moshpit/push-relay/sealbox"
)

func must[T any](v T, err error) T {
	if err != nil {
		panic(err)
	}
	return v
}

func main() {
	in := bufio.NewScanner(os.Stdin)
	in.Buffer(make([]byte, 1<<20), 1<<24)
	out := bufio.NewWriter(os.Stdout)
	defer out.Flush()
	for in.Scan() {
		f := strings.Split(in.Text(), " ")
		switch f[0] {
		case "seal":
			pt := must(hex.DecodeString(f[3]))
			var e sealbox.Envelope
			var err error
			if f[2] == "-" {
				e, err = sealbox.Seal(f[1], pt)
			} else {
				e, err = sealbox.SealWithIV(f[1], pt, f[2])
			}
			if err != nil {
				fmt.Fprintf(out, "err %v\n", err)
				continue
			}
			fmt.Fprintf(out, "%s\n", must(json.Marshal(e)))
		case "open":
			var e sealbox.Envelope
			if err := json.Unmarshal(must(hex.DecodeString(f[2])), &e); err != nil {
				fmt.Fprintf(out, "err %v\n", err)
				continue
			}
			pt, err := sealbox.Open(f[1], e)
			if err != nil {
				fmt.Fprintf(out, "err %v\n", err)
				continue
			}
			fmt.Fprintf(out, "ok %x\n", pt)
		case "mint":
			master := must(hex.DecodeString(f[1]))
			iat := must(strconv.ParseInt(f[4], 10, 64))
			fmt.Fprintf(out, "%s\n", mintSendToken(master, f[2], f[3], iat))
		case "verify":
			master := must(hex.DecodeString(f[1]))
			iat := must(strconv.ParseInt(f[5], 10, 64))
			now := must(strconv.ParseInt(f[6], 10, 64))
			ttl := must(strconv.ParseInt(f[7], 10, 64))
			ok := verifySendToken(master, f[2], f[3], f[4], iat, time.Unix(now, 0), time.Duration(ttl)*time.Second)
			fmt.Fprintf(out, "%v\n", ok)
		default:
			panic("unknown command " + f[0])
		}
	}
}
