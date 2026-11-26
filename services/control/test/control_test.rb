ENV["RACK_ENV"] = "test"
require "minitest/autorun"
require "rack/test"
require_relative "../app"

class ControlTest < Minitest::Test
  include Rack::Test::Methods
  def app = Control
  def setup
    Control.configure_storage(":memory:")
    Control.set :token, "a" * 32
    Control.set :engine, ->(method, path) { [200, {method: method, path: path}] }
    header "Host", "localhost"
  end
  def auth
    header "Authorization", "Bearer " + "a" * 32
  end
  def test_auth_and_engine
    get "/api/v1/status"
    assert_equal 401, last_response.status
    auth
    post "/api/v1/stream/start"
    assert_equal "/stream/start", JSON.parse(last_response.body)["path"]
  end
  def test_scene_revision_and_fail_closed
    auth
    header "Content-Type", "application/json"
    doc = {version: 1, name: "LT", layers: [{id: "hdmi", source: "hdmi", x: 0, y: 0, width: 1920, height: 1080, visible: true}]}
    body = JSON.generate(revision: 0, document: doc)
    put "/api/v1/scenes/lt", body
    assert_equal 200, last_response.status
    put "/api/v1/scenes/lt", body
    assert_equal 409, last_response.status
    doc[:layers][0][:width] = 1921
    put "/api/v1/scenes/lt", JSON.generate(revision: 1, document: doc)
    assert_equal 422, last_response.status
    post "/api/v1/program/transition"
    assert_equal 501, last_response.status
  end
end
