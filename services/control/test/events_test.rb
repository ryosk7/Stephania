# frozen_string_literal: true

require_relative "test_helper"

# Parses an SSE byte stream into [event, data] pairs (comments skipped).
module SSE
  def self.parse(text)
    text.split("\n\n").filter_map do |block|
      lines = block.lines(chomp: true).reject { |l| l.start_with?(":") || l.start_with?("retry:") }
      next if lines.empty?

      event = lines.find { |l| l.start_with?("event: ") }&.delete_prefix("event: ")
      data = lines.find { |l| l.start_with?("data: ") }&.delete_prefix("data: ")
      [event, data && JSON.parse(data)]
    end
  end
end

class EventHubTest < Minitest::Test
  def test_client_limit_and_release
    hub = EventHub.new
    subs = Array.new(EventHub::MAX_CLIENTS) { hub.subscribe }
    assert subs.all?
    assert_nil hub.subscribe
    hub.unsubscribe(subs.first)
    refute_nil hub.subscribe
  end

  def test_one_principal_cannot_take_every_slot
    hub = EventHub.new
    first, *rest = Array.new(EventHub::MAX_PER_PRINCIPAL + 1) { hub.subscribe(principal: "a") }
    assert first.closed?, "a new stream beyond the per-principal limit replaces that principal's oldest"
    refute rest.any?(&:closed?)
    assert_equal EventHub::MAX_PER_PRINCIPAL, hub.subscriber_count
    # One account over several sessions is bounded too.
    hub = EventHub.new
    subs = Array.new(EventHub::MAX_PER_USER + 1) { |i| hub.subscribe(user_id: 7, principal: "s#{i}", admin: false) }
    assert subs.first.closed?
    assert_equal EventHub::MAX_PER_USER, hub.subscriptions.count { |s| s.user_id == 7 }
  end

  def test_non_admins_leave_slots_for_admins
    hub = EventHub.new
    open = (1..EventHub::MAX_CLIENTS).map { |i| hub.subscribe(principal: "op#{i}", user_id: i, admin: false) }
    assert_equal EventHub::MAX_CLIENTS - EventHub::RESERVED_FOR_ADMIN, open.compact.size
    refute_nil hub.subscribe(principal: "admin", user_id: 99, admin: true)
  end

  def test_stream_ends_when_its_credential_is_no_longer_valid
    hub = EventHub.new
    valid = true
    sub = hub.subscribe(principal: "t", check: -> { valid })
    stream = EventHub::Stream.new(hub, sub, {}, heartbeat: 0.02, recheck: 0.05)
    reader = Thread.new { stream.each { |_| nil } }
    # Events keep flowing; the credential is still re-checked.
    pump = Thread.new { 200.times { hub.publish("audio.levels", {}) && sleep(0.005) } }
    sleep 0.15
    assert reader.alive?
    valid = false
    assert reader.join(2), "stream must end once the credential is revoked"
    stream.close
    pump.kill
    assert_equal 0, hub.subscriber_count
    raising = hub.subscribe(check: -> { raise "db down" })
    refute raising.valid?
  end

  def test_slow_consumer_is_dropped_without_blocking
    hub = EventHub.new
    slow = hub.subscribe
    fast = hub.subscribe
    EventHub::QUEUE_SIZE.times { |i| hub.publish("tick", {i: i}) }
    fast.queue.clear
    started = Time.now
    assert_equal 1, hub.publish("tick", {i: -1})
    assert_operator Time.now - started, :<, 0.5
    assert slow.closed?
    refute fast.closed?
    assert_equal 1, hub.subscriber_count
    assert_equal EventHub.frame("tick", {i: -1}), fast.queue.pop
  end

  def test_stream_sends_snapshot_events_and_keepalive_then_ends_on_close
    hub = EventHub.new
    sub = hub.subscribe
    stream = EventHub::Stream.new(hub, sub, {engine: nil, scenes: []}, heartbeat: 0.05)
    chunks = Queue.new
    reader = Thread.new { stream.each { |c| chunks << c } }
    hub.publish("scene.updated", {id: "a", revision: 1})
    sleep 0.15
    hub.unsubscribe(sub)
    assert reader.join(2), "stream must end once the subscription closes"
    text = Array.new(chunks.size) { chunks.pop }.join
    assert text.start_with?("retry: 2000\n\n")
    assert_includes text, ": keepalive\n\n"
    events = SSE.parse(text)
    assert_equal ["snapshot", {"engine" => nil, "scenes" => []}], events[0]
    assert_equal ["scene.updated", {"id" => "a", "revision" => 1}], events[1]
    stream.close
    assert_equal 0, hub.subscriber_count
  end
end

class EnginePollerTest < Minitest::Test
  # Engine double whose status is mutated by the test.
  class Engine
    attr_accessor :status

    def call(_method, _params = nil, request_id: nil)
      raise EngineClient::Unavailable, "down" unless status

      status
    end
  end

  def status(**over)
    {"instance_id" => "i1", "state" => "idle", "streaming_requested" => false,
     "native_worker_running" => false, "last_error" => "", "program" => nil}.merge(over.transform_keys(&:to_s))
  end

  def test_transitions_publish_once
    hub = EventHub.new
    sub = hub.subscribe
    engine = Engine.new
    poller = EventHub::EnginePoller.new(hub, -> { engine })

    poller.tick                      # first poll, engine down
    poller.tick                      # still down: no duplicate
    engine.status = status
    poller.tick                      # connected
    poller.tick                      # unchanged: nothing
    engine.status = status(state: "starting", streaming_requested: true)
    poller.tick
    engine.status = status(state: "starting", streaming_requested: true, program: {"scene_id" => "a", "revision" => 2})
    poller.tick
    engine.status = status(instance_id: "i2")
    poller.tick                      # restart + stream state back to idle + no program
    engine.status = nil
    poller.tick
    engine.status = status(instance_id: "i2")
    poller.tick                      # same process back after an outage
    engine.status = nil
    poller.tick
    engine.status = status(instance_id: "i3")
    poller.tick                      # new process after an outage

    events = SSE.parse(Array.new(sub.queue.size) { sub.queue.pop }.join)
    assert_equal %w[engine.unavailable engine.connected stream.state_changed program.changed engine.restarted stream.state_changed
                    engine.unavailable engine.connected engine.unavailable engine.restarted],
      events.map(&:first)
    assert_equal({"scene_id" => "a", "revision" => 2}, events[3][1])
    assert_equal "starting", events[2][1]["state"]
    assert_equal "i3", poller.last["instance_id"]
  end
end

class EventsEndpointTest < Minitest::Test
  include Rack::Test::Methods

  TOKEN = "e" * 32
  SCENE = {version: 1, name: "LT", layers: [{id: "hdmi", source: "hdmi", x: 0, y: 0, width: 1920, height: 1080, visible: true}]}.freeze

  def app = Control

  def setup
    Control.configure_storage(":memory:")
    Control.configure_events(interval: 0.05)
    Control.token = TOKEN
    @engine = FakeEngine.new do |req|
      FakeEngine.ok(req, {"instance_id" => "i1", "state" => "idle", "streaming_requested" => false,
                          "native_worker_running" => false, "last_error" => "", "program" => nil})
    end
    Control.engine = EngineClient.new(socket_path: @engine.path)
    header "Host", "localhost"
  end

  def teardown
    Control.poller.stop
    Control.events.close_all
    @engine.close
  end

  def env_for(path, token: TOKEN)
    Rack::MockRequest.env_for(path, "HTTP_HOST" => "localhost", "HTTP_AUTHORIZATION" => "Bearer #{token}", "REMOTE_ADDR" => "127.0.0.1")
  end

  def test_requires_token
    status, = Control.call(env_for("/api/v1/events", token: "x" * 32))
    assert_equal 401, status
  end

  def test_stream_delivers_snapshot_and_scene_updates
    status, headers, body = Control.call(env_for("/api/v1/events"))
    assert_equal 200, status
    assert_equal "text/event-stream", headers["Content-Type"] || headers["content-type"]
    chunks = Queue.new
    reader = Thread.new { body.each { |c| chunks << c } }

    header "Authorization", "Bearer #{TOKEN}"
    header "Content-Type", "application/json"
    put "/api/v1/scenes/main", JSON.generate(revision: 0, document: SCENE)
    assert_equal 200, last_response.status
    deadline = Time.now + 3
    text = +""
    until (%w[engine.connected scene.updated] - SSE.parse(text).map(&:first)).empty? || Time.now > deadline
      text << chunks.pop(timeout: 0.2).to_s
    end
    events = SSE.parse(text)
    assert_equal "snapshot", events.first[0]
    assert_includes events, ["scene.updated", {"id" => "main", "revision" => 1}]
    assert_includes events.map(&:first), "engine.connected"
  ensure
    body&.close
    reader&.join(2)
    assert_equal 0, Control.events.subscriber_count
  end

  def test_stream_limit_is_429
    # Fill the hub with other principals, then one more stream is refused.
    others = Array.new(EventHub::MAX_CLIENTS) { |i| Control.events.subscribe(principal: "other#{i}") }
    status, _, rejected = Control.call(env_for("/api/v1/events"))
    assert_equal 429, status
    assert_equal "too_many_streams", JSON.parse(rejected.each.to_a.join).dig("error", "code")
  ensure
    others&.each { |s| Control.events.unsubscribe(s) }
  end

  def test_same_token_replaces_its_oldest_stream
    bodies = Array.new(EventHub::MAX_PER_PRINCIPAL + 1) { Control.call(env_for("/api/v1/events")).tap { |r| assert_equal 200, r[0] }[2] }
    assert_equal EventHub::MAX_PER_PRINCIPAL, Control.events.subscriber_count
  ensure
    bodies&.each(&:close)
  end

  def test_stream_checks_follow_session_expiry_and_token_revocation
    Control.login_gate = Accounts::LoginGate.new
    header "Authorization", "Bearer #{TOKEN}"
    header "Content-Type", "application/json"
    post "/api/v1/users", JSON.generate(username: "sam", password: "correct horse battery", role: "operator")
    header "Authorization", nil
    post "/api/v1/auth/session", JSON.generate(username: "sam", password: "correct horse battery")
    cookie = last_response.headers["Set-Cookie"][/stephania_session=[^;]+/]
    status, _, body = Control.call(env_for("/api/v1/events").merge("HTTP_COOKIE" => cookie).tap { |e| e.delete("HTTP_AUTHORIZATION") })
    assert_equal 200, status
    sub = Control.events.subscriptions.last
    refute sub.admin
    assert sub.valid?
    Control.db[:sessions].update(last_seen_unix: Time.now.to_i - Accounts::IDLE_SECONDS - 1)
    refute sub.valid?, "an idle-expired session ends its stream"
    body.close

    dir = Dir.mktmpdir("stephania-events-lan")
    issued = "t3b_#{"3c" * 32}"
    tokens = File.join(dir, "tokens.json")
    File.write(tokens, JSON.generate(tokens: [{id: "x", sha256: OpenSSL::Digest::SHA256.hexdigest(issued)}]))
    Control.lan = LanAccess.new(pki_dir: dir, tokens_file: tokens)
    status, _, body = Control.call(env_for("/api/v1/events", token: issued))
    assert_equal 200, status
    sub = Control.events.subscriptions.last
    assert sub.valid?
    File.write(tokens, JSON.generate(tokens: []))
    File.utime(Time.now + 5, Time.now + 5, tokens)
    refute sub.valid?, "a revoked enrolled token ends its stream"
    body.close
  ensure
    Control.lan = LanAccess.new(pki_dir: nil, tokens_file: nil)
    FileUtils.remove_entry(dir) if dir
  end
end
