# frozen_string_literal: true

require_relative "test_helper"
require "zlib"

# Image assets, scene v2 references and Program transitions (docs/SCENE.md).
class AssetsTest < Minitest::Test
  include Rack::Test::Methods

  TOKEN = "s" * 32

  def app = Control

  # Minimal valid RGBA PNG.
  def png(width, height, rgba = [255, 0, 0, 255])
    chunk = ->(type, data) { [data.bytesize].pack("N") + type + data + [Zlib.crc32(type + data)].pack("N") }
    raw = (("\0".b + rgba.pack("C*") * width) * height)
    "\x89PNG\r\n\x1a\n".b + chunk.call("IHDR", [width, height, 8, 6, 0, 0, 0].pack("NNCCCCC")) +
      chunk.call("IDAT", Zlib::Deflate.deflate(raw)) + chunk.call("IEND", "")
  end

  def setup
    Control.configure_storage(":memory:")
    Control.token = TOKEN
    @dir = Dir.mktmpdir("t3box-assets-test")
    Control.assets_dir = @dir
    @applied = []
    @engine = FakeEngine.new do |req|
      @applied << req["params"] if req["method"] == "scene.apply"
      FakeEngine.ok(req, {"applied" => true, "scene_id" => req.dig("params", "scene_id"), "revision" => req.dig("params", "revision"), "first_frame_index" => 7})
    end
    Control.engine = EngineClient.new(socket_path: @engine.path)
    header "Host", "localhost"
    header "Authorization", "Bearer #{TOKEN}"
  end

  def teardown
    @engine.close
    FileUtils.rm_rf(@dir)
  end

  def body = JSON.parse(last_response.body)

  def upload(data, name = "logo.png")
    header "Content-Type", "image/png"
    post "/api/v1/assets?name=#{name}", data
    header "Content-Type", "application/json"
    last_response
  end

  def scene_with(asset)
    {version: 2, name: "Logo", layers: [
      {id: "h", type: "video", source: "hdmi", x: 0, y: 0, width: 1920, height: 1080, visible: true},
      {id: "l", type: "image", asset: asset, x: 1700, y: 960, width: 200, height: 100, visible: true, opacity: 80}
    ]}
  end

  def test_upload_is_content_addressed_group_readable_and_served
    data = png(4, 2)
    res = upload(data)
    assert_equal 201, res.status, res.body
    id = body["id"]
    assert_equal OpenSSL::Digest::SHA256.hexdigest(data)[0, 16], id
    assert_equal({"name" => "logo.png", "width" => 4, "height" => 2, "bytes" => data.bytesize}, body.slice("name", "width", "height", "bytes"))
    path = File.join(@dir, "#{id}.png")
    assert_equal data, File.binread(path)
    assert_equal 0o640, File.stat(path).mode & 0o777
    assert_equal 200, upload(data).status, "same content returns the existing asset"
    assert_equal 1, Control.db[:assets].count
    get "/api/v1/assets/#{id}"
    assert_equal 200, last_response.status
    assert_equal "image/png", last_response.content_type
    assert_equal data, last_response.body.b
    get "/api/v1/assets"
    assert_equal [id], body.map { |a| a["id"] }
  end

  def test_rejects_non_png_and_bad_dimensions
    assert_equal 422, upload("GIF89a....").status
    assert_equal "invalid_asset", body.dig("error", "code")
    huge = png(1, 1).dup
    huge[16, 8] = [5000, 1].pack("NN")
    assert_equal 422, upload(huge).status
    header "Content-Type", "image/jpeg"
    post "/api/v1/assets", "x"
    assert_equal 415, last_response.status
  end

  def test_scenes_must_reference_existing_assets_and_in_use_assets_cannot_be_deleted
    header "Content-Type", "application/json"
    put "/api/v1/scenes/logo", JSON.generate(revision: 0, document: scene_with("0123456789abcdef"))
    assert_equal 422, last_response.status
    assert_equal "unknown_asset", body.dig("error", "code")
    upload(png(2, 2))
    id = body["id"]
    put "/api/v1/scenes/logo", JSON.generate(revision: 0, document: scene_with(id))
    assert_equal 200, last_response.status, last_response.body
    delete "/api/v1/assets/#{id}"
    assert_equal 409, last_response.status
    assert_equal "asset_in_use", body.dig("error", "code")
    delete "/api/v1/scenes/logo?revision=1"
    delete "/api/v1/assets/#{id}"
    assert_equal 204, last_response.status
    refute File.exist?(File.join(@dir, "#{id}.png"))
  end

  def test_transition_is_validated_and_forwarded
    header "Content-Type", "application/json"
    put "/api/v1/scenes/lt", JSON.generate(revision: 0, document: {version: 1, name: "LT", layers: []})
    post "/api/v1/program/transition", JSON.generate(scene_id: "lt", revision: 1, transition: {type: "fade", duration_ms: 500})
    assert_equal 200, last_response.status, last_response.body
    assert_equal({"type" => "fade", "duration_ms" => 500}, @applied.last["transition"])
    post "/api/v1/program/transition", JSON.generate(scene_id: "lt", revision: 1)
    refute @applied.last.key?("transition"), "cut is the default"
    [{type: "fade", duration_ms: 0}, {type: "fade", duration_ms: 5001}, {type: "wipe"}, {type: "cut", duration_ms: 3}].each do |t|
      post "/api/v1/program/transition", JSON.generate(scene_id: "lt", revision: 1, transition: t)
      assert_equal 422, last_response.status, t.inspect
    end
  end

  def test_operator_can_view_but_not_upload
    Control.throttle = Accounts::Throttle.new
    header "Content-Type", "application/json"
    post "/api/v1/users", JSON.generate(username: "op", password: "operator password", role: "operator")
    header "Authorization", nil
    post "/api/v1/auth/session", JSON.generate(username: "op", password: "operator password")
    header "X-CSRF-Token", body["csrf_token"]
    get "/api/v1/assets"
    assert_equal 200, last_response.status
    assert_equal 403, upload(png(1, 1)).status
  end
end
