# frozen_string_literal: true

require "json"
require "socket"
require "securerandom"

# Client for the engine control IPC (docs/IPC.md, contracts/ipc.schema.json).
#
# Frames are a 4-byte big-endian length followed by UTF-8 JSON.
#
# Connections are pooled (at most MAX_CONNECTIONS, below the engine's limit
# of 8, ipc.rs MAX_CONNECTIONS) and reused for sequential requests: opening
# one connection per request under load ran into the engine's limit because
# a closed connection's slot is released only when the engine thread sees
# EOF (measured by ci/control-load.sh). Every request still has its own
# timeout, so a hung engine cannot pin a Puma thread longer than before; a
# connection that failed or timed out is discarded, never reused. Idle
# connections older than IDLE_SECONDS are dropped before the engine's own
# 5 s idle timeout, and a request that finds its pooled connection closed
# (engine restarted) is retried once on a fresh one — the engine closes
# idle connections only between requests, so the request was never read.
class EngineClient
  VERSION = 1
  MAX_FRAME_BYTES = 64 * 1024
  DEFAULT_SOCKET = "/run/t3box/engine.sock"

  # Engine could not be reached or violated the framing contract.
  class Unavailable < StandardError; end

  # A well-formed engine error response.
  class RemoteError < StandardError
    attr_reader :code

    def initialize(code, message)
      @code = code
      super(message)
    end
  end

  MAX_CONNECTIONS = 6
  IDLE_SECONDS = 3.0

  # The pooled connection was closed by the engine before it read anything.
  class Stale < StandardError; end

  attr_reader :socket_path

  def initialize(socket_path: DEFAULT_SOCKET, timeout: 2.0, max_connections: MAX_CONNECTIONS)
    @socket_path = socket_path
    @timeout = timeout
    @max = max_connections
    @idle = [] # [socket, last_used_monotonic]
    @open = 0
    @lock = Mutex.new
    @available = ConditionVariable.new
  end

  # Drop all idle connections (tests, shutdown).
  def close_idle
    @lock.synchronize do
      @idle.each { |sock, _| sock.close rescue nil }
      @open -= @idle.size
      @idle.clear
      @available.broadcast
    end
  end

  # Returns the `result` object or raises RemoteError / Unavailable.
  def call(method, params = nil, request_id: SecureRandom.hex(8))
    request = {version: VERSION, request_id: request_id, method: method}
    request[:params] = params unless params.nil?
    body = JSON.generate(request)
    raise ArgumentError, "request exceeds #{MAX_FRAME_BYTES} bytes" if body.bytesize > MAX_FRAME_BYTES

    response = exchange(body)
    unless response.is_a?(Hash) && response["version"] == VERSION
      raise Unavailable, "engine returned an unsupported response"
    end
    if response["ok"] == true && response["request_id"] == request_id && response["result"].is_a?(Hash)
      return response["result"]
    end
    error = response["error"]
    if response["ok"] == false && error.is_a?(Hash) && error["code"].is_a?(String)
      raise RemoteError.new(error["code"], error["message"].to_s)
    end
    raise Unavailable, "engine response did not match the request"
  end

  private

  def exchange(body)
    socket, reused = checkout
    healthy = false
    begin
      begin
        result = round_trip(socket, body, reused)
      rescue Stale
        socket.close rescue nil
        socket = connect
        result = round_trip(socket, body, false)
      end
      healthy = true
      result
    ensure
      checkin(socket, healthy)
    end
  rescue JSON::ParserError
    raise Unavailable, "engine returned invalid JSON"
  rescue SystemCallError, IOError => e
    raise Unavailable, "engine socket error: #{e.class.name.split('::').last}"
  end

  def round_trip(socket, body, reused)
    begin
      write_all(socket, [body.bytesize].pack("N") + body)
    rescue Errno::EPIPE, Errno::ECONNRESET
      raise Stale if reused

      raise
    end
    header = begin
      read_exact(socket, 4)
    rescue Unavailable => e
      raise Stale if reused && e.message == "engine closed the connection"

      raise
    end
    length = header.unpack1("N")
    raise Unavailable, "invalid engine frame length" if length.zero? || length > MAX_FRAME_BYTES

    JSON.parse(read_exact(socket, length).force_encoding(Encoding::UTF_8))
  end

  # [socket, reused?]; waits up to the timeout when all connections are busy.
  def checkout
    deadline = Process.clock_gettime(Process::CLOCK_MONOTONIC) + @timeout
    @lock.synchronize do
      loop do
        now = Process.clock_gettime(Process::CLOCK_MONOTONIC)
        while (entry = @idle.pop)
          sock, used = entry
          return [sock, true] if now - used < IDLE_SECONDS

          sock.close rescue nil
          @open -= 1
        end
        if @open < @max
          @open += 1
          break
        end
        remaining = deadline - now
        raise Unavailable, "engine connection pool exhausted" if remaining <= 0

        @available.wait(@lock, remaining)
      end
    end
    begin
      [connect, false]
    rescue StandardError
      @lock.synchronize { @open -= 1; @available.signal }
      raise
    end
  end

  def checkin(socket, healthy)
    @lock.synchronize do
      if healthy && socket && !socket.closed?
        @idle.push([socket, Process.clock_gettime(Process::CLOCK_MONOTONIC)])
      else
        socket&.close rescue nil
        @open -= 1
      end
      @available.signal
    end
  end

  def connect
    socket = Socket.new(:UNIX, :STREAM)
    begin
      socket.connect_nonblock(Socket.sockaddr_un(@socket_path))
    rescue IO::WaitWritable
      raise Unavailable, "engine connect timeout" unless socket.wait_writable(@timeout)

      begin
        socket.connect_nonblock(Socket.sockaddr_un(@socket_path))
      rescue Errno::EISCONN
        # connected
      end
    end
    socket
  rescue Unavailable, SystemCallError, ArgumentError => e
    socket&.close
    raise e if e.is_a?(Unavailable)

    raise Unavailable, "engine socket unavailable: #{e.class.name.split('::').last}"
  end

  def write_all(socket, data)
    until data.empty?
      begin
        written = socket.write_nonblock(data)
        data = data.byteslice(written..)
      rescue IO::WaitWritable
        raise Unavailable, "engine write timeout" unless socket.wait_writable(@timeout)
      end
    end
  end

  def read_exact(socket, length)
    buffer = +""
    while buffer.bytesize < length
      begin
        chunk = socket.read_nonblock(length - buffer.bytesize)
        buffer << chunk
      rescue IO::WaitReadable
        raise Unavailable, "engine read timeout" unless socket.wait_readable(@timeout)
      rescue EOFError
        raise Unavailable, "engine closed the connection"
      end
    end
    buffer
  end
end
