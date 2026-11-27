# frozen_string_literal: true

ENV["RACK_ENV"] = "test"
require "minitest/autorun"
require "rack/test"
require "socket"
require "tmpdir"
require "json"
require_relative "../app"

FIXTURES = File.expand_path("../../../contracts/fixtures", __dir__)

# Minimal engine speaking the real IPC framing on a Unix socket. The handler
# receives the decoded request and returns a response Hash, a raw String
# (written verbatim, for protocol-violation tests) or :hang.
class FakeEngine
  attr_reader :path, :requests

  def initialize(&handler)
    @dir = Dir.mktmpdir("stephania-fake-engine")
    @path = File.join(@dir, "engine.sock")
    @server = UNIXServer.new(@path)
    @requests = Queue.new
    @handler = handler
    @thread = Thread.new { loop { serve(@server.accept) } }
  end

  def serve(client)
    length = client.read(4)&.unpack1("N")
    return unless length

    request = JSON.parse(client.read(length))
    @requests << request
    reply = @handler.call(request)
    return sleep(5) if reply == :hang

    body = reply.is_a?(String) ? reply : JSON.generate(reply)
    client.write([body.bytesize].pack("N") + body)
  rescue IOError, SystemCallError
    nil
  ensure
    client&.close
  end

  def close
    @thread.kill
    @server.close
    FileUtils.remove_entry(@dir)
  end

  def self.ok(request, result)
    {"version" => 1, "request_id" => request["request_id"], "ok" => true, "result" => result}
  end

  def self.error(request, code, message = code)
    {"version" => 1, "request_id" => request["request_id"], "ok" => false, "error" => {"code" => code, "message" => message}}
  end
end
