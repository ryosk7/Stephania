# frozen_string_literal: true

require "puma/client"

# Single Puma process, bounded threads (docs/MASTER-ARCHITECTURE.md §16).
# Loopback only until trusted LAN TLS exists.
workers 0
# Each Server-Sent Events stream holds one thread for its lifetime, so the
# pool must exceed EventHub::MAX_CLIENTS (8) with headroom for requests.
max_threads = Integer(ENV.fetch("STEPHANIA_CONTROL_MAX_THREADS", 12))
raise ArgumentError, "STEPHANIA_CONTROL_MAX_THREADS must be >= 12 (8 event streams + 4 requests)" if max_threads < 12

threads Integer(ENV.fetch("STEPHANIA_CONTROL_MIN_THREADS", 1)), max_threads
bind "tcp://127.0.0.1:#{Integer(ENV.fetch('STEPHANIA_CONTROL_PORT', 9292))}"

# Trusted LAN: TLS only, with the device-CA certificate from stephania-pki.
# Explicitly configured but missing material is a startup failure, never a
# silent fallback to plain HTTP.
if (tls_port = ENV["STEPHANIA_CONTROL_TLS_PORT"]) && !tls_port.empty?
  pki = ENV.fetch("STEPHANIA_PKI_DIR", "/data/stephania/pki")
  cert = File.join(pki, "server.pem")
  key = File.join(pki, "server-key.pem")
  raise ArgumentError, "STEPHANIA_CONTROL_TLS_PORT set but #{cert} / #{key} missing (run stephania-pki ensure)" unless File.file?(cert) && File.file?(key)

  ssl_bind ENV.fetch("STEPHANIA_CONTROL_TLS_BIND", "0.0.0.0"), Integer(tls_port),
    cert: cert, key: key, verify_mode: "none", no_tlsv1: true, no_tlsv1_1: true
end
environment ENV.fetch("RACK_ENV", "production")
# Puma buffers a request body (to a Tempfile past 112 KB, on the RAM-backed
# /tmp) before the app sees it, so cap it here: 413 before reading, for any
# Content-Length above the largest body a route accepts (AssetStore::MAX_BYTES,
# image uploads). BodyLimit (lib/body_limit.rb) applies per-route limits.
http_content_length_limit 8 * 1024 * 1024
persistent_timeout 20
# Event streams never finish on their own; without a bound, a graceful stop
# (systemd TERM) would wait for every open Studio tab. Force-close them.
force_shutdown_after 2
first_data_timeout 10
max_fast_inline 4

# Puma 7 checks http_content_length_limit for chunked bodies only after the
# whole body has been written to its Tempfile. Abort the read as soon as the
# decoded body passes the limit (Puma answers a parser error with 400 and
# closes the connection).
module StephaniaChunkedBodyLimit
  def write_chunk(str)
    if @http_content_length_limit && @chunked_content_length + str.bytesize > @http_content_length_limit
      raise Puma::HttpParserError, "chunked request body exceeds #{@http_content_length_limit} bytes"
    end

    super
  end
end
Puma::Client.prepend(StephaniaChunkedBodyLimit) unless Puma::Client < StephaniaChunkedBodyLimit
