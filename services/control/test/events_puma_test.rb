# frozen_string_literal: true

# Server-Sent Events against a real Puma process (config/puma.rb), which is
# what proves chunks are flushed as they are produced, that disconnects free
# the stream slot, and that 8 open streams do not starve ordinary requests.
# Requires the bundled Puma; CI sets T3BOX_REQUIRE_PUMA=1 so it cannot skip.
require "minitest/autorun"
require "net/http"
require "json"
require "socket"
require "tmpdir"

class EventsPumaTest < Minitest::Test
  TOKEN = "p" * 32
  SCENE = {version: 1, name: "LT", layers: [{id: "hdmi", source: "hdmi", x: 0, y: 0, width: 1920, height: 1080, visible: true}]}.freeze
  ROOT = File.expand_path("..", __dir__)

  def setup
    begin
      Gem::Specification.find_by_name("puma")
    rescue Gem::MissingSpecError
      flunk "puma is required" if ENV["T3BOX_REQUIRE_PUMA"] == "1"
      skip "puma not installed"
    end
    @dir = Dir.mktmpdir("t3box-events-puma")
    @port = TCPServer.open("127.0.0.1", 0) { |s| s.addr[1] }
    env = {"T3BOX_CONTROL_TOKEN" => TOKEN, "T3BOX_CONTROL_PORT" => @port.to_s,
           "T3BOX_ENGINE_SOCKET" => File.join(@dir, "none.sock"), "T3BOX_CONTROL_DB" => File.join(@dir, "c.db"),
           "T3BOX_CONTROL_POLL_SECONDS" => "0.2", "RACK_ENV" => "production"}
    @log = File.join(@dir, "puma.log")
    @pid = Process.spawn(env, "bundle", "exec", "puma", "-C", "config/puma.rb", "config.ru", chdir: ROOT, out: @log, err: @log)
    deadline = Time.now + 20
    until listening?
      flunk "puma did not start:\n#{File.read(@log)}" if Time.now > deadline
      sleep 0.1
    end
  end

  def listening?
    TCPSocket.new("127.0.0.1", @port).close
    true
  rescue SystemCallError
    false
  end

  def teardown
    if @pid
      Process.kill("TERM", @pid)
      Process.wait(@pid)
    end
    FileUtils.remove_entry(@dir) if @dir
  end

  def request(verb, path, body = nil)
    http = Net::HTTP.new("127.0.0.1", @port)
    http.read_timeout = 5
    req = Net::HTTP.const_get(verb).new(path, "Authorization" => "Bearer #{TOKEN}", "Host" => "localhost", "Content-Type" => "application/json")
    req.body = JSON.generate(body) if body
    http.request(req)
  end

  # Opens a raw stream socket and returns it once the response head arrived.
  def open_stream
    sock = TCPSocket.new("127.0.0.1", @port)
    sock.write("GET /api/v1/events HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer #{TOKEN}\r\nAccept: text/event-stream\r\n\r\n")
    head = +""
    head << sock.readpartial(4096) until head.include?("\r\n\r\n")
    [sock, head]
  end

  # Status line of a fresh stream request; the socket is closed right away.
  def stream_status
    sock, head = open_stream
    head.lines.first
  ensure
    sock&.close
  end

  def read_until(sock, needle, buffer, timeout: 5)
    deadline = Time.now + timeout
    until buffer.include?(needle)
      remaining = deadline - Time.now
      flunk "timed out waiting for #{needle.inspect}; got #{buffer.inspect}" if remaining <= 0
      buffer << sock.readpartial(4096) if sock.wait_readable(remaining)
    end
    buffer
  end

  def test_stream_is_flushed_incrementally_and_bounded
    sock, head = open_stream
    assert_match %r{\AHTTP/1.1 200}, head
    assert_match %r{content-type: text/event-stream}i, head
    buffer = head.dup
    read_until(sock, "event: snapshot", buffer)
    # No engine on the socket path: the poller must announce it.
    read_until(sock, "event: engine.unavailable", buffer)

    assert_equal "200", request(:Put, "/api/v1/scenes/main", {revision: 0, document: SCENE}).code
    read_until(sock, %(event: scene.updated\ndata: {"id":"main","revision":1}), buffer)

    others = Array.new(7) { open_stream.first }
    assert_match %r{\AHTTP/1.1 429}, stream_status
    # Eight open streams must not starve ordinary requests.
    assert_equal "200", request(:Get, "/api/v1/scenes").code

    # Closing a client frees its slot (detected on the next write/keepalive
    # or event); publish to force a write.
    sock.close
    deadline = Time.now + 10
    loop do
      current = JSON.parse(request(:Get, "/api/v1/scenes/main").body)["revision"]
      request(:Put, "/api/v1/scenes/main", {revision: current, document: SCENE})
      break if stream_status.start_with?("HTTP/1.1 200")
      flunk "stream slot was not released" if Time.now > deadline
      sleep 0.2
    end
  ensure
    others&.each(&:close)
  end
end
