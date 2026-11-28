# frozen_string_literal: true

require_relative "test_helper"
require "openssl"

# Trusted-LAN rules (lib/lan_access.rb): TLS + certified Host + owner-issued
# token, independent of the loopback bootstrap token.
class LanAccessTest < Minitest::Test
  include Rack::Test::Methods

  BOOT = "b" * 32
  ISSUED = "t3b_#{"1f" * 32}"

  def app = Control

  def setup
    @dir = Dir.mktmpdir("stephania-lan")
    File.write(File.join(@dir, "endpoint.json"), JSON.generate(host: "stephania-ab12.local", port: 9443, ips: ["192.168.1.20", "fd00::5"],
      urls: [], ca_sha256: "0" * 64, not_after_unix: 2_000_000_000))
    @tokens = File.join(@dir, "tokens.json")
    write_tokens([ISSUED])
    Control.configure_storage(":memory:")
    Control.token = BOOT
    Control.lan = LanAccess.new(pki_dir: @dir, tokens_file: @tokens)
    @engine = FakeEngine.new { |req| FakeEngine.ok(req, {"state" => "idle"}) }
    Control.engine = EngineClient.new(socket_path: @engine.path)
  end

  def teardown
    @engine.close
    FileUtils.remove_entry(@dir)
    Control.lan = LanAccess.new(pki_dir: nil, tokens_file: nil)
  end

  def write_tokens(tokens, mtime: Time.now)
    File.write(@tokens, JSON.generate(tokens: tokens.map { |t| {id: "x", sha256: OpenSSL::Digest::SHA256.hexdigest(t), created_unix: 1} }))
    File.utime(mtime, mtime, @tokens)
  end

  def call(host:, https:, token:, origin: nil, extra: {})
    env = {"HTTP_HOST" => host, "HTTP_AUTHORIZATION" => "Bearer #{token}"}.merge(extra)
    env["HTTPS"] = "on" if https
    env["rack.url_scheme"] = https ? "https" : "http"
    env["HTTP_ORIGIN"] = origin if origin
    get "/api/v1/capabilities", {}, env
    [last_response.status, (JSON.parse(last_response.body).dig("error", "code") rescue nil)]
  end

  def test_ca_certificate_is_served_without_credentials_over_lan_tls_only
    File.write(File.join(@dir, "ca.pem"), "-----BEGIN CERTIFICATE-----\nAAAA\n-----END CERTIFICATE-----\n")
    get "/ca.pem", {}, {"HTTP_HOST" => "stephania-ab12.local", "HTTPS" => "on", "rack.url_scheme" => "https"}
    assert_equal 200, last_response.status
    assert_includes last_response.body, "BEGIN CERTIFICATE"
    assert_match(/attachment/, last_response.headers["Content-Disposition"])
    get "/ca.pem", {}, {"HTTP_HOST" => "stephania-ab12.local"}
    assert_equal 403, last_response.status, "plain HTTP on the LAN is refused"
  end

  def test_ca_certificate_in_der_hashes_to_the_pinned_fingerprint
    key = OpenSSL::PKey::EC.generate("prime256v1")
    cert = OpenSSL::X509::Certificate.new
    cert.version = 2
    cert.serial = 1
    cert.subject = cert.issuer = OpenSSL::X509::Name.parse("/CN=Stephania device CA stephania-ab12")
    cert.public_key = key
    cert.not_before = Time.now - 60
    cert.not_after = Time.now + 3600
    cert.sign(key, OpenSSL::Digest.new("SHA256"))
    File.write(File.join(@dir, "ca.pem"), cert.to_pem)
    get "/ca.crt", {}, {"HTTP_HOST" => "stephania-ab12.local", "HTTPS" => "on", "rack.url_scheme" => "https"}
    assert_equal 200, last_response.status
    assert_equal "application/pkix-cert", last_response.content_type
    assert_match(/stephania-ca\.crt/, last_response.headers["Content-Disposition"])
    assert_equal OpenSSL::Digest::SHA256.hexdigest(cert.to_der), OpenSSL::Digest::SHA256.hexdigest(last_response.body)
    get "/ca.crt", {}, {"HTTP_HOST" => "stephania-ab12.local"}
    assert_equal 403, last_response.status, "plain HTTP on the LAN is refused"
  end

  def test_lan_requires_tls_certified_host_and_issued_token
    assert_equal [403, "forbidden_host"], call(host: "stephania-ab12.local", https: false, token: ISSUED)
    assert_equal [403, "forbidden_host"], call(host: "evil.example", https: true, token: ISSUED)
    assert_equal [401, "unauthorized"], call(host: "stephania-ab12.local", https: true, token: BOOT), "bootstrap token is loopback-only"
    assert_equal 200, call(host: "stephania-ab12.local:9443", https: true, token: ISSUED).first
    assert_equal 200, call(host: "192.168.1.20", https: true, token: ISSUED).first
    assert_equal 200, call(host: "[fd00::5]:9443", https: true, token: ISSUED).first
    assert_equal "max-age=31536000", last_response.headers["Strict-Transport-Security"]
  end

  # Local vs LAN is decided by the listener and the TCP peer, never by Host
  # or X-Forwarded-*: the bootstrap token works only on the loopback listener.
  def test_bootstrap_token_is_refused_off_the_loopback_listener_whatever_the_headers
    lan = {"REMOTE_ADDR" => "192.168.1.50"}
    assert_equal [403, "forbidden_host"], call(host: "localhost", https: true, token: BOOT, extra: lan)
    assert_equal [403, "forbidden_host"], call(host: "localhost:9443", https: true, token: BOOT), "TLS listener even from loopback"
    assert_equal [401, "unauthorized"],
      call(host: "stephania-ab12.local", https: true, token: BOOT, extra: lan.merge("HTTP_X_FORWARDED_HOST" => "localhost"))
    assert_equal [401, "unauthorized"],
      call(host: "stephania-ab12.local", https: true, token: BOOT, extra: lan.merge("HTTP_FORWARDED" => "host=localhost;for=127.0.0.1"))
    assert_equal [403, "forbidden_host"], call(host: "localhost", https: false, token: BOOT, extra: lan), "plain HTTP from a LAN peer"
    assert_equal [403, "forbidden_host"],
      call(host: "localhost", https: false, token: BOOT, extra: lan.merge("HTTP_X_FORWARDED_FOR" => "127.0.0.1"))
    assert_equal [403, "forbidden_host"], call(host: "evil.example", https: false, token: BOOT), "DNS rebinding on loopback"
    assert_equal [403, "forbidden_host"], call(host: "evil.example", https: false, token: BOOT, extra: {"HTTP_X_FORWARDED_HOST" => "localhost"})
    assert_equal 200, call(host: "stephania-ab12.local", https: true, token: ISSUED, extra: lan).first
  end

  def test_loopback_accepts_bootstrap_and_issued_tokens
    assert_equal 200, call(host: "localhost", https: false, token: BOOT).first
    assert_equal 200, call(host: "127.0.0.1", https: false, token: ISSUED).first
    assert_nil last_response.headers["Strict-Transport-Security"]
    assert_equal [401, "unauthorized"], call(host: "localhost", https: false, token: "t3b_#{"00" * 32}")
  end

  def test_cross_origin_is_rejected
    assert_equal [403, "forbidden_origin"], call(host: "stephania-ab12.local:9443", https: true, token: ISSUED, origin: "https://evil.example")
    assert_equal 200, call(host: "stephania-ab12.local:9443", https: true, token: ISSUED, origin: "https://stephania-ab12.local:9443").first
  end

  def test_revocation_and_missing_files_fail_closed
    assert_equal 200, call(host: "stephania-ab12.local", https: true, token: ISSUED).first
    write_tokens([], mtime: Time.now + 5) # factory reset clears tokens
    assert_equal [401, "unauthorized"], call(host: "stephania-ab12.local", https: true, token: ISSUED)
    File.write(@tokens, "{not json")
    File.utime(Time.now + 10, Time.now + 10, @tokens)
    assert_equal [401, "unauthorized"], call(host: "stephania-ab12.local", https: true, token: ISSUED)
    File.delete(File.join(@dir, "endpoint.json"))
    assert_equal [403, "forbidden_host"], call(host: "stephania-ab12.local", https: true, token: ISSUED)
  end

  def test_issued_tokens_alone_configure_the_api
    Control.token = nil
    assert_equal 200, call(host: "localhost", https: false, token: ISSUED).first
    Control.lan = LanAccess.new(pki_dir: nil, tokens_file: nil)
    assert_equal [503, "not_configured"], call(host: "localhost", https: false, token: ISSUED)
  end
end
