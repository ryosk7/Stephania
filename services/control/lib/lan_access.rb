# frozen_string_literal: true

require "json"
require "openssl"

# Trusted-LAN access state shared with the device core:
# - endpoint.json from t3box-pki (crate::pki): the device's certified
#   host name and addresses, the only non-loopback Host values accepted,
#   and only over TLS;
# - tokens.json from t3box-provision (crate::management_tokens): SHA-256
#   hashes of owner-issued tokens, the only credentials accepted on the LAN.
# Both files are re-read when their mtime changes; anything missing or
# malformed means "no LAN access" (fail closed).
class LanAccess
  def initialize(pki_dir:, tokens_file:)
    @endpoint_path = pki_dir && File.join(pki_dir, "endpoint.json")
    @tokens_path = tokens_file
    @mutex = Mutex.new
    @cache = {}
  end

  def configured? = !@tokens_path.nil?

  # Hosts (lowercase, IPv6 without brackets) certified for LAN TLS.
  def hosts
    endpoint = load(@endpoint_path) { |v| v if v.is_a?(Hash) }
    return [] unless endpoint

    [endpoint["host"], *Array(endpoint["ips"])].grep(String).map(&:downcase)
  end

  def lan_host?(host) = hosts.include?(host.to_s.downcase.delete_prefix("[").delete_suffix("]"))

  def issued_token?(supplied)
    return false unless supplied.is_a?(String) && supplied.start_with?("t3b_") && supplied.bytesize == 68

    digest = OpenSSL::Digest::SHA256.hexdigest(supplied)
    hashes = load(@tokens_path) { |v| Array(v["tokens"]).filter_map { |t| t["sha256"] if t.is_a?(Hash) && t["sha256"].is_a?(String) } if v.is_a?(Hash) } || []
    # Compare against every entry so timing does not reveal which matched.
    hashes.map { |h| h.bytesize == digest.bytesize && OpenSSL.fixed_length_secure_compare(h, digest) }.any?
  end

  private

  def load(path)
    return nil unless path

    @mutex.synchronize do
      stat = File.stat(path)
      key = [stat.mtime.to_r, stat.size, stat.ino]
      cached = @cache[path]
      return cached[1] if cached && cached[0] == key

      value = yield(JSON.parse(File.read(path, 65_536)))
      @cache[path] = [key, value]
      value
    end
  rescue SystemCallError, JSON::ParserError
    @mutex.synchronize { @cache.delete(path) }
    nil
  end
end
