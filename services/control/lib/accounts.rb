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
# - Login throttling: per username and per client address (the TCP peer),
#   5 failures lock for 60 s, doubling up to 15 min. Unknown users cost the
#   same scrypt work as known ones; LoginGate bounds that work overall.
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

  # Whether the session with this stored digest is still valid, without
  # touching last_seen (an open event stream is not activity). Event streams
  # re-check this so they end when the session expires or the user is
  # disabled.
  def session_live?(db, token_sha256, now = Time.now.to_i)
    session = db[:sessions].where(token_sha256: token_sha256).first
    return false unless session && session[:expires_at_unix] > now && session[:last_seen_unix] + IDLE_SECONDS > now

    user = db[:users].where(id: session[:user_id]).first
    !user.nil? && !user[:disabled]
  end

  def admins_remaining(db, excluding_id)
    db[:users].where(role: "admin", disabled: false).exclude(id: excluding_id).count
  end

  # In-memory failure counter keyed by username and by client address.
  # Memory is bounded (MAX_KEYS keys of at most MAX_KEY_BYTES). A full table
  # evicts the least recently failed key that is not locked; a key whose
  # lock is still running is never evicted. When every key is locked, keys
  # not in the table are refused until the first lock expires (fail
  # closed), so filling the table cannot unlock or bypass anything.
  class Throttle
    LIMIT = 5
    BASE_LOCK = 60
    MAX_LOCK = 900
    MAX_KEYS = 4096
    MAX_KEY_BYTES = 128

    def initialize
      @mutex = Mutex.new
      # Insertion order is kept as least recently failed first.
      @state = {}
    end

    def size = @mutex.synchronize { @state.size }

    # Seconds until the key may try again (0 = allowed).
    def wait(keys, now = Time.now.to_f)
      keys = keys.map { |k| bound(k) }
      @mutex.synchronize do
        waits = keys.map { |k| (s = @state[k]) ? [s[:until] - now, 0].max : 0 }
        if keys.any? { |k| !@state.key?(k) } && @state.size >= MAX_KEYS && !evictable(now)
          waits << @state.each_value.map { |s| s[:until] }.min - now
        end
        waits.max.ceil
      end
    end

    def failure(keys, now = Time.now.to_f)
      keys = keys.map { |k| bound(k) }
      @mutex.synchronize do
        keys.each do |k|
          s = @state.delete(k)
          if s.nil?
            if @state.size >= MAX_KEYS
              victim = evictable(now)
              next unless victim

              @state.delete(victim)
            end
            s = {failures: 0, until: 0, lock: BASE_LOCK}
          end
          @state[k] = s
          s[:failures] += 1
          next if s[:failures] < LIMIT

          s[:until] = now + s[:lock]
          s[:lock] = [s[:lock] * 2, MAX_LOCK].min
          s[:failures] = 0
        end
      end
    end

    def success(keys)
      @mutex.synchronize { keys.each { |k| @state.delete(bound(k)) } }
    end

    private

    def bound(key) = key.byteslice(0, MAX_KEY_BYTES)

    # Least recently failed key whose lock is not running, or nil.
    def evictable(now) = @state.each_key.find { |k| @state[k][:until] <= now }
  end

  # Bounds password-hash work. OpenSSL::KDF.scrypt holds the GVL for its
  # whole run, so unbounded sign-ins would stall every other request
  # (stream stop, transitions, event streams). At most MAX_CONCURRENT
  # verifications run at once and RATE per second on average (BURST at
  # once); beyond that a sign-in is refused (429 login_busy) without work.
  class LoginGate
    MAX_CONCURRENT = 2
    RATE = 2.0
    BURST = 10

    def initialize(rate: RATE, burst: BURST, max_concurrent: MAX_CONCURRENT)
      @rate = rate
      @burst = burst
      @max_concurrent = max_concurrent
      @mutex = Mutex.new
      @tokens = burst.to_f
      @at = nil
      @active = 0
    end

    # true when a verification may start now; pair with #leave.
    def enter(now = clock)
      @mutex.synchronize do
        @tokens = [@tokens + ((now - (@at || now)).clamp(0, nil) * @rate), @burst].min
        @at = now
        return false if @active >= @max_concurrent || @tokens < 1

        @tokens -= 1
        @active += 1
        true
      end
    end

    def leave = @mutex.synchronize { @active -= 1 }

    private

    def clock = Process.clock_gettime(Process::CLOCK_MONOTONIC)
  end
end
