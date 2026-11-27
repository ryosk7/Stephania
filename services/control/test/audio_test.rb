# frozen_string_literal: true

require_relative "test_helper"

class AudioApiTest < Minitest::Test
  include Rack::Test::Methods

  TOKEN = "u" * 32

  def app = Control

  def setup
    Control.configure_storage(":memory:")
    Control.token = TOKEN
    @configured = []
    @engine = FakeEngine.new do |req|
      case req["method"]
      when "audio.configure"
        @configured << req["params"]
        if req.dig("params", "inputs").any? { |i| i["device"] == "engine-rejects" }
          FakeEngine.error(req, "invalid_params", "device rejected")
        else
          FakeEngine.ok(req, {"running" => false, "inputs" => req.dig("params", "inputs")})
        end
      when "audio.status"
        FakeEngine.ok(req, {"running" => true, "master" => {"peak_dbfs" => -6.0}, "inputs" => [
          {"id" => "hdmi", "enabled" => true, "device" => "plughw:1,0", "channels" => 2, "gain_db" => 0.0, "muted" => false, "state" => "active"},
          {"id" => "mic", "enabled" => false, "device" => "", "channels" => 1, "gain_db" => 0.0, "muted" => false, "state" => "disabled"}
        ]})
      when "engine.status" then FakeEngine.ok(req, {"instance_id" => "i1", "state" => "idle"})
      end
    end
    Control.engine = EngineClient.new(socket_path: @engine.path)
    header "Host", "localhost"
    header "Authorization", "Bearer #{TOKEN}"
    header "Content-Type", "application/json"
  end

  def teardown = @engine.close
  def body = JSON.parse(last_response.body)

  def test_partial_update_is_validated_saved_and_applied
    put "/api/v1/audio", JSON.generate(inputs: [{id: "mic", enabled: true, device: "plughw:CARD=Mic,DEV=0", gain_db: -6}])
    assert_equal 200, last_response.status, last_response.body
    assert body["applied"]
    mic = body["stored"].find { |i| i["id"] == "mic" }
    assert_equal({"id" => "mic", "enabled" => true, "device" => "plughw:CARD=Mic,DEV=0", "channels" => 1, "gain_db" => -6.0, "muted" => false}, mic)
    assert_equal %w[hdmi mic], @configured.last["inputs"].map { |i| i["id"] }.sort, "the full configuration is applied"
    put "/api/v1/audio", JSON.generate(inputs: [{id: "mic", muted: true}])
    assert_equal 200, last_response.status
    stored = Control.db[:audio_inputs].where(id: "mic").first
    assert stored[:muted]
    assert_equal(-6.0, stored[:gain_db], "omitted fields keep their value")
    get "/api/v1/audio"
    assert_equal(-6.0, body["engine"]["master"]["peak_dbfs"])
  end

  def test_partial_update_starts_from_the_engine_defaults
    put "/api/v1/audio", JSON.generate(inputs: [{id: "hdmi", gain_db: -2}])
    assert_equal 200, last_response.status, last_response.body
    hdmi = body["stored"].find { |i| i["id"] == "hdmi" }
    assert_equal "plughw:1,0", hdmi["device"]
  end

  def test_rejects_invalid_configurations_without_saving
    [
      {inputs: [{id: "aux"}]},
      {inputs: [{id: "hdmi", gain_db: 13}]},
      {inputs: [{id: "hdmi", device: "hw:0;reboot"}]},
      {inputs: [{id: "mic", enabled: true}]},
      {inputs: [{id: "mic", enabled: true, device: "x"}, {id: "hdmi", device: "x"}]},
      {inputs: [{id: "mic", channels: 4}]},
      {inputs: []},
      {other: 1}
    ].each do |bad|
      put "/api/v1/audio", JSON.generate(bad)
      assert_equal 422, last_response.status, bad.inspect
      assert_equal "invalid_audio", body.dig("error", "code")
    end
    put "/api/v1/audio", JSON.generate(inputs: [{id: "hdmi", device: "engine-rejects"}])
    assert_equal 422, last_response.status
    assert_equal 0, Control.db[:audio_inputs].count
  end

  def test_saved_while_engine_down_and_reapplied_on_reconnect
    Control.engine = EngineClient.new(socket_path: File.join(Dir.tmpdir, "t3box-none-#{Process.pid}.sock"))
    put "/api/v1/audio", JSON.generate(inputs: [{id: "hdmi", gain_db: -3}])
    assert_equal 422, last_response.status, "without the engine the HDMI device must be given"
    put "/api/v1/audio", JSON.generate(inputs: [{id: "hdmi", device: "plughw:1,0", gain_db: -3}])
    assert_equal 200, last_response.status
    refute body["applied"]
    Control.engine = EngineClient.new(socket_path: @engine.path)
    Control.configure_events(interval: 60)
    Control.poller.tick
    assert_equal(-3.0, @configured.last["inputs"].find { |i| i["id"] == "hdmi" }["gain_db"], "applied when the engine appears")
  end

  def test_camera_role_cannot_change_audio
    header "Authorization", "Bearer #{TOKEN}"
    post "/api/v1/credentials/camera", JSON.generate(ttl_s: 600, label: "cam")
    cam = body["token"]
    header "Authorization", "Bearer #{cam}"
    put "/api/v1/audio", JSON.generate(inputs: [{id: "hdmi", muted: true}])
    assert_equal 403, last_response.status
  end

  def test_levels_are_published_only_while_running
    hub = EventHub.new
    sub = hub.subscribe
    EventHub::LevelPoller.new(hub, -> { Control.engine }).tick
    frame = sub.queue.pop(timeout: 1)
    assert_includes frame, "event: audio.levels"
  end
end
