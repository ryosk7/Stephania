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
environment ENV.fetch("RACK_ENV", "production")
persistent_timeout 20
first_data_timeout 10
max_fast_inline 4
