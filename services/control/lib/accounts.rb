# frozen_string_literal: true

require "openssl"
require "securerandom"
require "base64"

# Studio user accounts and cookie sessions (docs/AUTH.md).
#
# - Passwords: scrypt (N=2^14, r=8, p=1), per-user random salt, encoded as
#   "scrypt$N$r$p$salt$hash" so parameters can be raised later.
# - Sessions: 256-bit random cookie value, stored only as SHA-256; a
#   separate CSRF token must accompany every state-changing request.
#   12 h idle timeout, 7 day absolute lifetime, at most 8 per user.
# - Login throttling: per username and per client address, 5 failures lock
#   for 60 s, doubling up to 15 min. Unknown users cost the same scrypt
#   work as known ones.
module Accounts
  ROLES = %w[admin operator].freeze
  USERNAME = /\A[a-z0-9][a-z0-9_.-]{0,31}\z/
  MIN_PASSWORD = 10
  MAX_PASSWORD = 256
  IDLE_SECONDS = 12 * 3600
  MAX_AGE_SECONDS = 7 * 86_400
  MAX_SESSIONS_PER_USER = 8
  TOUCH_EVERY = 60
  COOKIE = "stephania_session"
  SCRYPT = {N: 2**14, r: 8, p: 1}.freeze
  DUMMY_HASH = "scrypt$16384$8$1$#{Base64.strict_encode64("\0" * 16)}$#{Base64.strict_encode64("\0" * 32)}"

  module_function

  def hash_password(password)
    salt = SecureRandom.random_bytes(16)
    digest = OpenSSL::KDF.scrypt(password, salt: salt, length: 32, **SCRYPT)
    ["scrypt", SCRYPT[:N], SCRYPT[:r], SCRYPT[:p], Base64.strict_encode64(salt), Base64.strict_encode64(digest)].join("$")
  end

  def verify_password(password, encoded)
    kind, n, r, p, salt, expected = encoded.to_s.split("$")
    return false unless kind == "scrypt" && salt && expected

    expected = Base64.strict_decode64(expected)
    actual = OpenSSL::KDF.scrypt(password.to_s, salt: Base64.strict_decode64(salt), length: expected.bytesize,
      N: Integer(n), r: Integer(r), p: Integer(p))
    OpenSSL.fixed_length_secure_compare(actual, expected)
  rescue ArgumentError
    false
  end

  # nil when acceptable, otherwise a message.
  def password_problem(password)
    return "password must be a string" unless password.is_a?(String)
    return "password must be at least #{MIN_PASSWORD} characters" if password.length < MIN_PASSWORD
    return "password must be at most #{MAX_PASSWORD} characters" if password.length > MAX_PASSWORD

    nil
  end

  def user_json(row)
    {id: row[:id], username: row[:username], role: row[:role], disabled: row[:disabled], created_at_unix: row[:created_at_unix]}
  end

  def digest(value) = OpenSSL::Digest::SHA256.hexdigest(value)

  # Returns [cookie_value, session_row].
  def create_session(db, user_id, now = Time.now.to_i)
    value = SecureRandom.urlsafe_base64(32)
    row = {token_sha256: digest(value), user_id: user_id, csrf_token: SecureRandom.urlsafe_base64(32),
           created_at_unix: now, last_seen_unix: now, expires_at_unix: now + MAX_AGE_SECONDS}
    db.transaction do
      db[:sessions].where(Sequel[:expires_at_unix] <= now).delete
      db[:sessions].insert(row)
      keep = db[:sessions].where(user_id: user_id).reverse(:last_seen_unix).limit(MAX_SESSIONS_PER_USER).select(:token_sha256)
      db[:sessions].where(user_id: user_id).exclude(token_sha256: keep).delete
    end
    [value, row]
  end

  # [session_row, user_row] for a live session cookie, or nil.
  def lookup_session(db, value, now = Time.now.to_i)
    return nil unless value.is_a?(String) && value.match?(/\A[A-Za-z0-9_-]{43}\z/)

    session = db[:sessions].where(token_sha256: digest(value)).first
    return nil unless session
    if session[:expires_at_unix] <= now || session[:last_seen_unix] + IDLE_SECONDS <= now
      db[:sessions].where(token_sha256: session[:token_sha256]).delete
      return nil
    end
    user = db[:users].where(id: session[:user_id]).first
    return nil if user.nil? || user[:disabled]

    if now - session[:last_seen_unix] >= TOUCH_EVERY
      db[:sessions].where(token_sha256: session[:token_sha256]).update(last_seen_unix: now)
    end
    [session, user]
  end

  def admins_remaining(db, excluding_id)
    db[:users].where(role: "admin", disabled: false).exclude(id: excluding_id).count
  end

  # In-memory failure counter keyed by username and by client address.
  class Throttle
    LIMIT = 5
    BASE_LOCK = 60
    MAX_LOCK = 900
    MAX_KEYS = 4096

    def initialize
      @mutex = Mutex.new
      @state = {}
    end

    # Seconds until the key may try again (0 = allowed).
    def wait(keys, now = Time.now.to_f)
      @mutex.synchronize do
        keys.map { |k| (s = @state[k]) ? [s[:until] - now, 0].max : 0 }.max.ceil
      end
    end

    def failure(keys, now = Time.now.to_f)
      @mutex.synchronize do
        @state.shift while @state.size >= MAX_KEYS
        keys.each do |k|
          s = (@state[k] ||= {failures: 0, until: 0, lock: BASE_LOCK})
          s[:failures] += 1
          next if s[:failures] < LIMIT

          s[:until] = now + s[:lock]
          s[:lock] = [s[:lock] * 2, MAX_LOCK].min
          s[:failures] = 0
        end
      end
    end

    def success(keys)
      @mutex.synchronize { keys.each { |k| @state.delete(k) } }
    end
  end
end
