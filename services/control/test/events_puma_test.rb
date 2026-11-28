# frozen_string_literal: true

# Server-Sent Events against a real Puma process (config/puma.rb), which is
# what proves chunks are flushed as they are produced, that disconnects free
# the stream slot, and that 8 open streams do not starve ordinary requests.
# Requires the bundled Puma; CI sets STEPHANIA_REQUIRE_PUMA=1 so it cannot skip.
require "minitest/autorun"
require "net/http"
require "json"
require "socket"
require "tmpdir"
require "openssl"

class EventsPumaTest < Minitest::Test
  TOKEN = "p" * 32
  # Enrolled owner tokens (also accepted on loopback): one principal each.
  ISSUED = Array.new(8) { |i| "t3b_#{format("%02x", i) * 32}" }
  SCENE = {version: 1, name: "LT", layers: [{id: "hdmi", source: "hdmi", x: 0, y: 0, width: 1920, height: 1080, visible: true}]}.freeze
  ROOT = File.expand_path("..", __dir__)

  def setup
    begin
      Gem::Specification.find_by_name("puma")
    rescue Gem::MissingSpecError
      flunk "puma is required" if ENV["STEPHANIA_REQUIRE_PUMA"] == "1"
      skip "puma not installed"
    end
    @dir = Dir.mktmpdir("stephania-events-puma")
    @port = TCPServer.open("127.0.0.1", 0) { |s| s.addr[1] }
    tokens = File.join(@dir, "tokens.json")
    File.write(tokens, JSON.generate(tokens: ISSUED.map { |t| {id: t[4, 4], sha256: OpenSSL::Digest::SHA256.hexdigest(t)} }))
    env = {"STEPHANIA_CONTROL_TOKEN" => TOKEN, "STEPHANIA_CONTROL_PORT" => @port.to_s, "STEPHANIA_CONTROL_TOKENS_FILE" => tokens,
           "STEPHANIA_ENGINE_SOCKET" => File.join(@dir, "none.sock"), "STEPHANIA_CONTROL_DB" => File.join(@dir, "c.db"),
           "STEPHANIA_CONTROL_POLL_SECONDS" => "0.2", "RACK_ENV" => "production"}
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
  def open_stream(token = TOKEN)
    sock = TCPSocket.new("127.0.0.1", @port)
    sock.write("GET /api/v1/events HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer #{token}\r\nAccept: text/event-stream\r\n\r\n")
    head = +""
    head << sock.readpartial(4096) until head.include?("\r\n\r\n")
    [sock, head]
  end

  # Status line of a fresh stream request; the socket is closed right away.
  def stream_status(token = TOKEN)
    sock, head = open_stream(token)
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

  def test_open_streams_do_not_block_shutdown
    sock, head = open_stream
    read_until(sock, "event: snapshot", head.dup)
    started = Time.now
    Process.kill("TERM", @pid)
    deadline = started + 8
    sleep 0.1 until Process.wait(@pid, Process::WNOHANG) || Time.now > deadline
    exited = Time.now <= deadline
    Process.kill("KILL", @pid) unless exited
    Process.wait(@pid) unless exited
    @pid = nil
    assert exited, "puma ignored TERM with an open event stream:\n#{File.read(@log)}"
  ensure
    sock&.close
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

    others = ISSUED.first(7).map { |t| open_stream(t).first }
    assert_match %r{\AHTTP/1.1 429}, stream_status(ISSUED.last)
    # Eight open streams must not starve ordinary requests.
    assert_equal "200", request(:Get, "/api/v1/scenes").code

    # Closing a client frees its slot (detected on the next write/keepalive
    # or event); publish to force a write.
    sock.close
    deadline = Time.now + 10
    loop do
      current = JSON.parse(request(:Get, "/api/v1/scenes/main").body)["revision"]
      request(:Put, "/api/v1/scenes/main", {revision: current, document: SCENE})
      break if stream_status(ISSUED.last).start_with?("HTTP/1.1 200")
      flunk "stream slot was not released" if Time.now > deadline
      sleep 0.2
    end
  ensure
    others&.each(&:close)
  end

  def test_one_token_cannot_hold_every_stream
    first, head = open_stream
    buffer = read_until(first, "event: snapshot", head.dup)
    later = Array.new(3) { open_stream.first }
    # The fourth stream of the same credential replaces the oldest, whose
    # response ends (last chunk).
    read_until(first, "\r\n0\r\n\r\n", buffer)
    assert_match %r{\AHTTP/1.1 200}, stream_status(ISSUED.first), "other principals still get a stream"
  ensure
    first&.close
    later&.each(&:close)
  end

  # Puma refuses bodies above http_content_length_limit before reading them,
  # and chunked bodies as soon as they pass it (config/puma.rb).
  def test_oversized_bodies_are_refused_before_buffering
    sock = TCPSocket.new("127.0.0.1", @port)
    sock.write("POST /api/v1/auth/session HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\n" \
               "Content-Length: #{(8 * 1024 * 1024) + 1}\r\n\r\n")
    head = +""
    head << sock.readpartial(4096) until head.include?("\r\n\r\n")
    assert_match %r{\AHTTP/1.1 413}, head
    sock.close

    sock = TCPSocket.new("127.0.0.1", @port)
    sock.write("POST /api/v1/auth/session HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\n\r\n")
    chunk = "#{(1024 * 1024).to_s(16)}\r\n#{"x" * 1024 * 1024}\r\n"
    sent = 0
    response = +""
    begin
      20.times do
        sock.write(chunk)
        sent += 1
      end
      response << sock.readpartial(4096) while sock.wait_readable(2) && !response.include?("\r\n\r\n")
    rescue SystemCallError, IOError
      nil # the server dropped the connection mid-body (reset: its 400 may be lost)
    end
    assert sent < 20 || response.start_with?("HTTP/1.1 400"), "20 MiB chunked body was accepted: #{response.inspect}"
    # Still serving.
    assert_equal "200", request(:Get, "/api/v1/scenes").code
  ensure
    sock&.close
  end
end
