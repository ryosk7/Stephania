# frozen_string_literal: true

# Single Puma process, bounded threads (docs/MASTER-ARCHITECTURE.md §16).
# Loopback only until trusted LAN TLS exists.
workers 0
# Each Server-Sent Events stream holds one thread for its lifetime, so the
# pool must exceed EventHub::MAX_CLIENTS (8) with headroom for requests.
max_threads = Integer(ENV.fetch("T3BOX_CONTROL_MAX_THREADS", 12))
raise ArgumentError, "T3BOX_CONTROL_MAX_THREADS must be >= 12 (8 event streams + 4 requests)" if max_threads < 12

threads Integer(ENV.fetch("T3BOX_CONTROL_MIN_THREADS", 1)), max_threads
bind "tcp://127.0.0.1:#{Integer(ENV.fetch('T3BOX_CONTROL_PORT', 9292))}"

# Trusted LAN: TLS only, with the device-CA certificate from t3box-pki.
# Explicitly configured but missing material is a startup failure, never a
# silent fallback to plain HTTP.
if (tls_port = ENV["T3BOX_CONTROL_TLS_PORT"]) && !tls_port.empty?
  pki = ENV.fetch("T3BOX_PKI_DIR", "/data/t3box/pki")
  cert = File.join(pki, "server.pem")
  key = File.join(pki, "server-key.pem")
  raise ArgumentError, "T3BOX_CONTROL_TLS_PORT set but #{cert} / #{key} missing (run t3box-pki ensure)" unless File.file?(cert) && File.file?(key)

  ssl_bind ENV.fetch("T3BOX_CONTROL_TLS_BIND", "0.0.0.0"), Integer(tls_port),
    cert: cert, key: key, verify_mode: "none", no_tlsv1: true, no_tlsv1_1: true
end
environment ENV.fetch("RACK_ENV", "production")
persistent_timeout 20
# Event streams never finish on their own; without a bound, a graceful stop
# (systemd TERM) would wait for every open Studio tab. Force-close them.
force_shutdown_after 2
first_data_timeout 10
max_fast_inline 4
