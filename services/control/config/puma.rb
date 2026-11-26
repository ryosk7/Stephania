# frozen_string_literal: true

# Single Puma process, bounded threads (docs/MASTER-ARCHITECTURE.md §16).
# Loopback only until trusted LAN TLS exists.
workers 0
threads Integer(ENV.fetch("T3BOX_CONTROL_MIN_THREADS", 1)), Integer(ENV.fetch("T3BOX_CONTROL_MAX_THREADS", 4))
bind "tcp://127.0.0.1:#{Integer(ENV.fetch('T3BOX_CONTROL_PORT', 9292))}"
environment ENV.fetch("RACK_ENV", "production")
persistent_timeout 20
first_data_timeout 10
max_fast_inline 4
