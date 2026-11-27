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
    @dir = Dir.mktmpdir("t3box-lan")
    File.write(File.join(@dir, "endpoint.json"), JSON.generate(host: "t3box-ab12.local", port: 9443, ips: ["192.168.1.20", "fd00::5"],
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

  def call(host:, https:, token:, origin: nil)
    env = {"HTTP_HOST" => host, "HTTP_AUTHORIZATION" => "Bearer #{token}"}
    env["HTTPS"] = "on" if https
    env["rack.url_scheme"] = https ? "https" : "http"
    env["HTTP_ORIGIN"] = origin if origin
    get "/api/v1/capabilities", {}, env
    [last_response.status, (JSON.parse(last_response.body).dig("error", "code") rescue nil)]
  end

  def test_ca_certificate_is_served_without_credentials_over_lan_tls_only
    File.write(File.join(@dir, "ca.pem"), "-----BEGIN CERTIFICATE-----\nAAAA\n-----END CERTIFICATE-----\n")
    get "/ca.pem", {}, {"HTTP_HOST" => "t3box-ab12.local", "HTTPS" => "on", "rack.url_scheme" => "https"}
    assert_equal 200, last_response.status
    assert_includes last_response.body, "BEGIN CERTIFICATE"
    assert_match(/attachment/, last_response.headers["Content-Disposition"])
    get "/ca.pem", {}, {"HTTP_HOST" => "t3box-ab12.local"}
    assert_equal 403, last_response.status, "plain HTTP on the LAN is refused"
  end

  def test_lan_requires_tls_certified_host_and_issued_token
    assert_equal [403, "forbidden_host"], call(host: "t3box-ab12.local", https: false, token: ISSUED)
    assert_equal [403, "forbidden_host"], call(host: "evil.example", https: true, token: ISSUED)
    assert_equal [401, "unauthorized"], call(host: "t3box-ab12.local", https: true, token: BOOT), "bootstrap token is loopback-only"
    assert_equal 200, call(host: "t3box-ab12.local:9443", https: true, token: ISSUED).first
    assert_equal 200, call(host: "192.168.1.20", https: true, token: ISSUED).first
    assert_equal 200, call(host: "[fd00::5]:9443", https: true, token: ISSUED).first
    assert_equal "max-age=31536000", last_response.headers["Strict-Transport-Security"]
  end

  def test_loopback_accepts_bootstrap_and_issued_tokens
    assert_equal 200, call(host: "localhost", https: false, token: BOOT).first
    assert_equal 200, call(host: "127.0.0.1", https: false, token: ISSUED).first
    assert_nil last_response.headers["Strict-Transport-Security"]
    assert_equal [401, "unauthorized"], call(host: "localhost", https: false, token: "t3b_#{"00" * 32}")
  end

  def test_cross_origin_is_rejected
    assert_equal [403, "forbidden_origin"], call(host: "t3box-ab12.local:9443", https: true, token: ISSUED, origin: "https://evil.example")
    assert_equal 200, call(host: "t3box-ab12.local:9443", https: true, token: ISSUED, origin: "https://t3box-ab12.local:9443").first
  end

  def test_revocation_and_missing_files_fail_closed
    assert_equal 200, call(host: "t3box-ab12.local", https: true, token: ISSUED).first
    write_tokens([], mtime: Time.now + 5) # factory reset clears tokens
    assert_equal [401, "unauthorized"], call(host: "t3box-ab12.local", https: true, token: ISSUED)
    File.write(@tokens, "{not json")
    File.utime(Time.now + 10, Time.now + 10, @tokens)
    assert_equal [401, "unauthorized"], call(host: "t3box-ab12.local", https: true, token: ISSUED)
    File.delete(File.join(@dir, "endpoint.json"))
    assert_equal [403, "forbidden_host"], call(host: "t3box-ab12.local", https: true, token: ISSUED)
  end

  def test_issued_tokens_alone_configure_the_api
    Control.token = nil
    assert_equal 200, call(host: "localhost", https: false, token: ISSUED).first
    Control.lan = LanAccess.new(pki_dir: nil, tokens_file: nil)
    assert_equal [503, "not_configured"], call(host: "localhost", https: false, token: ISSUED)
  end
end
