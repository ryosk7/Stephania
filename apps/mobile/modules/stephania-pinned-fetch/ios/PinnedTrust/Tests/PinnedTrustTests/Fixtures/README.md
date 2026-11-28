Public certificates only (no keys), issued by `stephania-pki ensure --test-loopback --host stephania-ci --ip 127.0.0.1`
in two separate directories (`ca`/`leaf` and `other-ca`/`other-leaf`). Tests pin the
evaluation date, so expiry of the fixtures never breaks them.
