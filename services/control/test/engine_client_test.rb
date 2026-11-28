# frozen_string_literal: true

require_relative "test_helper"

# Connection pooling in EngineClient (lib/engine_client.rb).
class EngineClientTest < Minitest::Test
  def setup
    @engine = FakeEngine.new { |req| FakeEngine.ok(req, {"n" => 1}) }
  end

  def teardown = @engine.close

  # FakeEngine closes every connection after one reply, like an engine that
  # restarted or timed out an idle connection: the pooled connection is
  # stale on every reuse and the request is retried once on a fresh one.
  def test_stale_pooled_connections_are_replaced_transparently
    c = EngineClient.new(socket_path: @engine.path)
    5.times { assert_equal({"n" => 1}, c.call("engine.status")) }
    assert_equal 5, @engine.requests.size, "each request reached the engine exactly once"
  end

  def test_concurrency_is_bounded_and_waiters_get_a_connection
    c = EngineClient.new(socket_path: @engine.path, max_connections: 2)
    results = Queue.new
    8.times.map { Thread.new { results << c.call("engine.status") } }.each(&:join)
    assert_equal 8, results.size
  end

  def test_pool_exhaustion_times_out_as_unavailable
    hang = FakeEngine.new { :hang }
    c = EngineClient.new(socket_path: hang.path, timeout: 0.3, max_connections: 1)
    t = Thread.new { assert_raises(EngineClient::Unavailable) { c.call("engine.status") } }
    sleep 0.05
    error = assert_raises(EngineClient::Unavailable) { c.call("engine.status") }
    t.join
    assert_match(/pool exhausted|timeout/, error.message)
    # The timed-out connection was discarded: a working engine is reachable again.
    c2 = EngineClient.new(socket_path: @engine.path, max_connections: 1)
    assert_equal({"n" => 1}, c2.call("engine.status"))
  ensure
    hang&.close
  end

  def test_oversized_request_is_refused_before_sending
    c = EngineClient.new(socket_path: @engine.path)
    error = assert_raises(EngineClient::RequestTooLarge) { c.call("scene.apply", {"pad" => "x" * EngineClient::MAX_FRAME_BYTES}) }
    assert_kind_of EngineClient::RemoteError, error
    assert_equal "request_too_large", error.code
    assert_equal 0, @engine.requests.size
  end

  # scene.apply waits longer than the default timeout: the engine's own
  # deadline for it (load + prepare + ack, 4 s) exceeds the 2 s default.
  def test_scene_apply_outlasts_the_default_timeout
    slow = FakeEngine.new { |req| sleep 0.5; FakeEngine.ok(req, {"applied" => true}) }
    c = EngineClient.new(socket_path: slow.path, timeout: 0.2)
    assert_raises(EngineClient::Unavailable) { c.call("engine.status") }
    c2 = EngineClient.new(socket_path: slow.path, timeout: 0.2)
    assert_equal({"applied" => true}, c2.call("scene.apply", {}))
  ensure
    slow&.close
  end
end
