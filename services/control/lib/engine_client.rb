# frozen_string_literal: true

require "json"
require "socket"
require "securerandom"

# Client for the engine control IPC (docs/IPC.md, contracts/ipc.schema.json).
#
# Frames are a 4-byte big-endian length followed by UTF-8 JSON. One request is
# sent per connection so a hung engine can never pin a Puma thread beyond the
# configured timeout, and no long-lived state has to be reconciled after an
# engine restart.
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

  attr_reader :socket_path

  def initialize(socket_path: DEFAULT_SOCKET, timeout: 2.0)
    @socket_path = socket_path
    @timeout = timeout
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
    socket = connect
    begin
      write_all(socket, [body.bytesize].pack("N") + body)
      header = read_exact(socket, 4)
      length = header.unpack1("N")
      raise Unavailable, "invalid engine frame length" if length.zero? || length > MAX_FRAME_BYTES

      JSON.parse(read_exact(socket, length).force_encoding(Encoding::UTF_8))
    ensure
      socket.close
    end
  rescue JSON::ParserError
    raise Unavailable, "engine returned invalid JSON"
  rescue SystemCallError, IOError => e
    raise Unavailable, "engine socket error: #{e.class.name.split('::').last}"
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
