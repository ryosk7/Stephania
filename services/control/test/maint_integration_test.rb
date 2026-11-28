# frozen_string_literal: true

# Runs /api/v1/maintenance against the real stephania-maint binary
# (docs/MAINTENANCE.md). Enabled when STEPHANIA_MAINT_BIN points at it.
# sshd is not started (systemctl is a stand-in); ci/maint-ssh-e2e.sh covers
# logging in with the real OpenSSH.
require_relative "test_helper"
require "base64"
require "openssl"

class MaintIntegrationTest < Minitest::Test
  include Rack::Test::Methods

  TOKEN = "i" * 32

  def app = Control

  def setup
    bin = ENV["STEPHANIA_MAINT_BIN"]
    skip "STEPHANIA_MAINT_BIN not set" unless bin && File.executable?(bin)
    @dir = Dir.mktmpdir("stephania-maint")
    pki = File.join(@dir, "pki")
    FileUtils.mkdir_p(pki)
    File.write(File.join(pki, "endpoint.json"), JSON.generate(host: "stephania-it.local", port: 9443, ips: ["192.168.1.9"], urls: [],
      ca_sha256: "0" * 64, not_after_unix: 4_102_444_800))
    @socket = File.join(@dir, "maint.sock")
    @log = File.join(@dir, "maint.log")
    env = {"STEPHANIA_MAINT_DIR" => File.join(@dir, "maint"), "STEPHANIA_MAINT_SOCKET" => @socket,
           "STEPHANIA_PKI_DIR" => pki, "STEPHANIA_MAINT_SYSTEMCTL" => "true"}
    @pid = Process.spawn(env, bin, out: @log, err: @log)
    50.times { break if File.socket?(@socket); sleep 0.1 }
    assert File.socket?(@socket), "stephania-maint did not listen: #{File.read(@log)}"
    Control.configure_storage(":memory:")
    Control.token = TOKEN
    Control.maint = EngineClient.new(socket_path: @socket, timeout: 15.0, max_connections: 2)
    header "Host", "localhost"
    header "Content-Type", "application/json"
    header "Authorization", "Bearer #{TOKEN}"
    # A LAN client: stephania-maint refuses loopback and the device's own
    # addresses (192.168.1.9 in endpoint.json).
    env "REMOTE_ADDR", "192.168.1.20"
  end

  def teardown
    if @pid
      Process.kill("TERM", @pid)
      Process.wait(@pid)
    end
    FileUtils.remove_entry(@dir) if @dir
  end

  def body = JSON.parse(last_response.body)

  # RFC 6238 code from the device's secret, as Connect computes it.
  def code(offset = 0)
    secret = [JSON.parse(File.read(File.join(@dir, "maint", "state.json")))["otp_secret"]].pack("H*")
    mac = OpenSSL::HMAC.digest("SHA1", secret, [(Time.now.to_i / 30) + offset].pack("Q>"))
    o = mac.getbyte(19) & 15
    format("%06d", (mac[o, 4].unpack1("N") & 0x7fffffff) % 1_000_000)
  end

  def ssh_public_key
    raw = OpenSSL::PKey.generate_key("ED25519").raw_public_key
    blob = ["ssh-ed25519".bytesize].pack("N") + "ssh-ed25519" + [raw.bytesize].pack("N") + raw
    "ssh-ed25519 #{Base64.strict_encode64(blob)} it"
  end

  def test_window_certificate_and_close_through_control
    post "/api/v1/maintenance/window", JSON.generate(code: code == "000000" ? "111111" : "000000")
    assert_equal [401, "invalid_code"], [last_response.status, body.dig("error", "code")]

    post "/api/v1/maintenance/window", JSON.generate(code: code)
    assert_equal 201, last_response.status, last_response.body
    window = body
    assert_equal "maint", window["user"]

    get "/api/v1/maintenance"
    assert_equal [true, "owner"], [body["open"], body.dig("window", "actor")]

    post "/api/v1/maintenance/certificate", JSON.generate(window_id: window["window_id"], public_key: ssh_public_key)
    assert_equal 200, last_response.status, last_response.body
    assert body["certificate"].start_with?("ssh-ed25519-cert-v01@openssh.com ")
    assert body["host_ca"].start_with?("ssh-ed25519 ")
    assert_equal({"user" => "maint", "host" => "localhost", "port" => 22}, body.slice("user", "host", "port"))
    assert File.read(File.join(@dir, "maint", "ssh_host_ed25519_key-cert.pub")).start_with?("ssh-ed25519-cert-v01@openssh.com ")

    delete "/api/v1/maintenance/window"
    assert_equal 204, last_response.status
    post "/api/v1/maintenance/certificate", JSON.generate(window_id: window["window_id"], public_key: ssh_public_key)
    assert_equal [409, "window_closed"], [last_response.status, body.dig("error", "code")]
    log = File.read(@log)
    assert_includes log, '"event":"maintenance.certificate_issued"'
    assert_includes log, '"actor":"owner"'
  end

  def test_loopback_and_the_device_itself_are_refused
    ["127.0.0.1", "::1", "192.168.1.9"].each do |addr|
      post "/api/v1/maintenance/window", JSON.generate(code: code), {"REMOTE_ADDR" => addr}
      assert_equal [422, "invalid_params"], [last_response.status, body.dig("error", "code")], addr
    end
    post "/api/v1/maintenance/window", JSON.generate(code: code)
    assert_equal 201, last_response.status, last_response.body
    window = body
    post "/api/v1/maintenance/certificate", JSON.generate(window_id: window["window_id"], public_key: ssh_public_key), {"REMOTE_ADDR" => "192.168.1.30"}
    assert_equal [403, "not_window_opener"], [last_response.status, body.dig("error", "code")]
    post "/api/v1/maintenance/certificate", JSON.generate(window_id: window["window_id"], public_key: ssh_public_key), {"REMOTE_ADDR" => "127.0.0.1"}
    assert_equal [422, "invalid_params"], [last_response.status, body.dig("error", "code")]
    assert File.file?(File.join(@dir, "maint", "revoked.krl"))
  end
end
