# Two stages, distroless-ish result: the relay is one static binary and a CA
# bundle (it dials api.push.apple.com over TLS and must be able to verify it).
FROM golang:1.24-alpine AS build
WORKDIR /src
COPY . .
# CGO off so the binary is genuinely static and runs on scratch.
RUN CGO_ENABLED=0 GOOS=linux go build -trimpath -ldflags="-s -w" -o /relay .

FROM alpine:3.20
RUN apk add --no-cache ca-certificates \
 && adduser -D -H -u 10001 relay \
 && mkdir -p /var/lib/moshpit-relay \
 && chown relay /var/lib/moshpit-relay
USER relay
COPY --from=build /relay /relay
EXPOSE 8080
ENTRYPOINT ["/relay"]
