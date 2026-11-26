# frozen_string_literal: true

# Runs the control client against the real Rust engine binary.
# Enabled when T3BOX_ENGINE_BIN points at a built `t3box` executable.
require_relative "test_helper"

class EngineIntegrationTest < Minitest::Test
  def setup
    @bin = ENV["T3BOX_ENGINE_BIN"]
    skip "T3BOX_ENGINE_BIN not set" unless @bin && File.executable?(@bin)
    @dir = Dir.mktmpdir("t3box-engine")
    @socket = File.join(@dir, "engine.sock")
    @log = File.join(@dir, "engine.log")
    env = {"T3BOX_ENGINE_SOCKET" => @socket, "T3BOX_OUTPUT" => ""}
    @pid = Process.spawn(env, @bin, out: @log, err: @log)
    50.times { break if File.socket?(@socket); sleep 0.1 }
    assert File.socket?(@socket), "engine did not create socket: #{File.read(@log)}"
    @client = EngineClient.new(socket_path: @socket)
  end

  def teardown
    return unless @pid

    Process.kill("TERM", @pid)
    Process.wait(@pid)
    FileUtils.remove_entry(@dir)
  end

  def test_status_capabilities_and_fail_closed_paths
    status = @client.call("engine.status")
    assert_equal "idle", status["state"]
    assert_equal false, status["youtube_ingest_confirmed"]
    caps = @client.call("engine.capabilities")
    assert_equal 1, caps["ipc_version"]
    assert_equal false, caps["program_apply"]
    error = assert_raises(EngineClient::RemoteError) { @client.call("stream.start") }
    assert_equal "engine_start_failed", error.code
    assert_equal false, @client.call("stream.stop")["streaming_requested"]
  end

  def test_shared_ipc_fixtures_against_real_engine
    Dir[File.join(FIXTURES, "ipc", "*.json")].sort.each do |file|
      fixture = JSON.parse(File.read(file))
      request = fixture["request"]
      # stream.start depends on configuration; covered above.
      next if request["method"] == "stream.start"

      raw = JSON.generate(request)
      UNIXSocket.open(@socket) do |s|
        s.write([raw.bytesize].pack("N") + raw)
        response = JSON.parse(s.read(s.read(4).unpack1("N")))
        assert_equal fixture.dig("expect", "ok"), response["ok"], File.basename(file)
        if (code = fixture.dig("expect", "code"))
          assert_equal code, response.dig("error", "code"), File.basename(file)
        end
      end
    end
  end

  def test_control_app_end_to_end
    Control.configure_storage(":memory:")
    Control.token = "t" * 32
    Control.engine = @client
    session = Rack::Test::Session.new(Rack::MockSession.new(Control))
    session.header "Host", "localhost"
    session.header "Authorization", "Bearer #{"t" * 32}"
    session.get "/api/v1/status"
    assert_equal 200, session.last_response.status
    assert_equal "idle", JSON.parse(session.last_response.body)["state"]
    session.post "/api/v1/stream/start"
    assert_equal 503, session.last_response.status
    assert_equal "engine_start_failed", JSON.parse(session.last_response.body).dig("error", "code")
  end
end
