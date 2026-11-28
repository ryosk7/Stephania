# frozen_string_literal: true

require "json"
require "securerandom"

# Request body limits enforced before Sinatra touches the body. Sinatra's
# dispatch! parses form and multipart bodies (Rack copies file parts into
# more Tempfiles) before any before-filter runs, so without this an
# unauthenticated client could make control parse arbitrary bodies.
#
# - Each route has a byte limit: `default` unless listed in `routes`
#   ([verb, path] => bytes, matched on the raw path, so an encoded spelling
#   of a listed path only ever gets the smaller default).
# - multipart/form-data is refused everywhere: no route takes it.
#
# Puma's http_content_length_limit (config/puma.rb) caps what is buffered
# at all; this is the per-route check on top of it.
class BodyLimit
  def initialize(app, default:, routes: {})
    @app = app
    @default = default
    @routes = routes
  end

  def call(env)
    if env["CONTENT_TYPE"].to_s.downcase.start_with?("multipart/")
      return error(415, "unsupported_media_type", "multipart bodies are not accepted")
    end

    limit = @routes.fetch([env["REQUEST_METHOD"], env["PATH_INFO"]], @default)
    length = env["CONTENT_LENGTH"]
    if length && !length.empty? && (!length.match?(/\A\d+\z/) || length.to_i > limit)
      return error(413, "body_too_large", "request body exceeds #{limit} bytes")
    end

    @app.call(env)
  end

  private

  def error(status, code, message)
    id = SecureRandom.hex(8)
    [status, {"content-type" => "application/json", "cache-control" => "no-store", "x-request-id" => id},
     [JSON.generate(error: {code: code, message: message, request_id: id})]]
  end
end
