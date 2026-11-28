# frozen_string_literal: true

require "json"
require "securerandom"

# The stream destination (YouTube RTMPS ingest URL + stream key), set by an
# admin through PUT /api/v1/output (docs/AUTH.md, contracts/openapi.yaml).
#
# Stored as its own file, not in control.db: the database is copied into
# update snapshots, and the stream key must not travel with it. The file is
# written atomically with mode 0600 in control's state directory (0700,
# stephania-control, on /data), so only the control service can read it.
# The engine never reads it: control sends the full output URL with each
# `stream.start` over the engine socket (docs/IPC.md).
#
# The stream key never leaves this class except through #output_url (for
# stream.start). Every API view is #public_json: an admin sees the server
# URL and the last 4 characters of the key, an operator only `configured`.
class OutputStore
  # rtmps://host[:port]/app, mirroring crate::rtmps_tls::Target::parse (the
  # engine validates again): a DNS name with a dot, not an IPv4 literal.
  URL = %r{\Artmps://(?<host>[A-Za-z0-9.-]{1,253})(?::(?<port>[0-9]{1,5}))?/(?<app>[A-Za-z0-9._~-]{1,128})\z}
  LABEL = /\A[A-Za-z0-9](?:[A-Za-z0-9-]{0,61}[A-Za-z0-9])?\z/
  # YouTube keys are like abcd-efgh-ijkl-mnop-qrst; at least 8 characters
  # so key_last4 never shows most of a key.
  KEY = /\A[A-Za-z0-9._~-]{8,256}\z/
  LAST_CHARS = 4

  class Invalid < StandardError; end

  # `path` nil keeps the output in memory only (tests, a control without a
  # state directory).
  def initialize(path)
    @path = path
    @lock = Mutex.new
    @memory = nil
  end

  attr_reader :path

  # Validates and stores; returns the stored record. Raises Invalid with a
  # message that never contains the key.
  def save(url, stream_key, now: Time.now.to_i)
    raise Invalid, "url must be rtmps://host[:port]/app (for YouTube: rtmps://a.rtmps.youtube.com/live2)" unless self.class.valid_url?(url)
    raise Invalid, "stream_key must be 8 to 256 characters: letters, digits, '-', '_', '.', '~'" unless stream_key.is_a?(String) && stream_key.match?(KEY)

    record = {"url" => url, "stream_key" => stream_key, "updated_at_unix" => now}
    @lock.synchronize { @path ? write_file(record) : @memory = record }
    record
  end

  def clear
    @lock.synchronize do
      @memory = nil
      File.delete(@path) if @path && File.exist?(@path)
    end
  end

  # The stored record or nil. An unreadable or malformed file counts as
  # not configured (and is reported as such) rather than failing requests.
  def load
    @lock.synchronize do
      next @memory unless @path
      next nil unless File.file?(@path)

      record = JSON.parse(File.read(@path))
      valid = record.is_a?(Hash) && self.class.valid_url?(record["url"]) && record["stream_key"].is_a?(String) &&
        record["stream_key"].match?(KEY)
      valid ? record : nil
    end
  rescue JSON::ParserError, SystemCallError
    nil
  end

  # Full destination for stream.start, or nil when none is stored.
  def output_url
    record = load
    record && "#{record["url"]}/#{record["stream_key"]}"
  end

  # API view. Never includes the stream key; admins get its last 4
  # characters (key_last4) to recognise which key is set.
  def public_json(admin:)
    record = load
    return {configured: !record.nil?} unless admin
    return {configured: false, url: nil, key_last4: nil, updated_at_unix: nil} unless record

    {configured: true, url: record["url"], key_last4: record["stream_key"][-LAST_CHARS..],
     updated_at_unix: record["updated_at_unix"]}
  end

  # STEPHANIA_CONTROL_OUTPUT_FILE, else output.json beside a file-backed
  # control.db (control's state directory), else nil (memory only).
  def self.path_from_env(env)
    explicit = env["STEPHANIA_CONTROL_OUTPUT_FILE"]
    return explicit unless explicit.nil? || explicit.empty?

    db = env.fetch("STEPHANIA_CONTROL_DB", ":memory:")
    db == ":memory:" ? nil : File.join(File.dirname(db), "output.json")
  end

  def self.valid_url?(url)
    m = url.is_a?(String) && URL.match(url)
    return false unless m

    host = m[:host]
    return false unless host.include?(".") && !host.match?(/\A[0-9.]+\z/) && host.split(".", -1).all? { |l| l.match?(LABEL) }

    m[:port].nil? || (1..65_535).cover?(m[:port].to_i)
  end

  private

  # Atomic replace: a new 0600 file beside the old one, fsync, rename.
  def write_file(record)
    dir = File.dirname(@path)
    tmp = File.join(dir, ".#{File.basename(@path)}.#{SecureRandom.hex(6)}.tmp")
    File.open(tmp, File::WRONLY | File::CREAT | File::EXCL, 0o600) do |f|
      f.write(JSON.generate(record))
      f.flush
      f.fsync
    end
    File.rename(tmp, @path)
    File.open(dir) { |d| d.fsync rescue nil }
  ensure
    File.delete(tmp) if tmp && File.exist?(tmp)
  end
end
