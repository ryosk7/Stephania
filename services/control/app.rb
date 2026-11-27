# frozen_string_literal: true

require "sinatra/base"
require "json"
require "sequel"
require "openssl"
require "securerandom"
require_relative "lib/engine_client"
require_relative "lib/scene_validator"
require_relative "lib/event_hub"
require_relative "lib/lan_access"

# T3Box management plane. Contract: contracts/openapi.yaml.
#
# Never carries media. Talks to the engine only through EngineClient
# (Unix socket IPC). Plain HTTP is loopback-only (T3BOX_CONTROL_TOKEN).
# The LAN is served only over TLS with the device-CA certificate
# (crate::pki) and accepts only owner-issued tokens enrolled over
# authenticated BLE (crate::management_tokens); see LanAccess.
class Control < Sinatra::Base
  MAX_BODY_BYTES = 65_536
  MAX_REVISIONS_KEPT = 50
  MIGRATIONS = File.expand_path("db/migrations", __dir__)
  ID = /\A[a-zA-Z0-9_-]{1,64}\z/
  ALLOWED_HOSTS = %w[localhost 127.0.0.1].freeze

  # Engine IPC error code -> HTTP status. Unknown codes are a contract
  # violation by the engine and surface as 502.
  ENGINE_STATUS = {
    "engine_start_failed" => 503,
    "busy" => 503,
    "invalid_scene" => 422,
    "not_implemented" => 501,
    "program_inactive" => 409,
    "program_timeout" => 503
  }.freeze

  set :bind, "127.0.0.1"
  set :public_folder, File.expand_path("../../apps/studio/dist", __dir__)
  set :show_exceptions, false
  set :raise_errors, false
  set :dump_errors, false
  set :logging, false
  set :host_authorization, {permitted_hosts: []} if Sinatra::VERSION.to_i >= 4

  # Dependencies are plain class attributes, not Sinatra settings: `set`
  # treats any Proc value as a lazily evaluated getter and calls it with no
  # arguments, which is what turned the injected engine adapter into an
  # HTTP 500 in the first foundation CI run.
  class << self
    attr_accessor :db, :token, :engine, :events, :poller, :lan

    def configure_storage(path)
      database = Sequel.sqlite(path, max_connections: 1, timeout: 1000)
      database.run("PRAGMA foreign_keys = ON")
      database.run("PRAGMA journal_mode = WAL") unless path == ":memory:"
      Sequel.extension :migration
      Sequel::Migrator.run(database, MIGRATIONS)
      self.db = database
      @engine_instance = nil
    end

    # Restart reconciliation: remember the last engine instance seen and
    # report whether it changed. Thread-safe under multi-threaded Puma.
    def observe_engine_instance(instance_id)
      @engine_mutex ||= Mutex.new
      @engine_mutex.synchronize do
        previous = @engine_instance
        @engine_instance = instance_id
        !previous.nil? && previous != instance_id
      end
    end

    def configure_from_env(env = ENV)
      self.token = env["T3BOX_CONTROL_TOKEN"]
      self.lan = LanAccess.new(pki_dir: env["T3BOX_PKI_DIR"], tokens_file: env["T3BOX_CONTROL_TOKENS_FILE"])
      self.engine = EngineClient.new(socket_path: env.fetch("T3BOX_ENGINE_SOCKET", EngineClient::DEFAULT_SOCKET))
      configure_storage(env.fetch("T3BOX_CONTROL_DB", ":memory:"))
      configure_events(interval: Float(env.fetch("T3BOX_CONTROL_POLL_SECONDS", "1.0")))
    end

    # The poller reads `Control.engine` on every tick, so tests that swap
    # the engine adapter are observed without restarting it.
    def configure_events(interval: 1.0)
      poller&.stop
      events&.close_all
      self.events = EventHub.new
      self.poller = EventHub::EnginePoller.new(events, -> { Control.engine }, interval: interval)
    end
  end

  configure { configure_from_env }

  helpers do
    def request_id
      @request_id ||= SecureRandom.hex(8)
    end

    def json_reply(value, code = 200)
      status code
      JSON.generate(value)
    end

    # Consistent error envelope: {"error":{"code","message","request_id"}}.
    def fail_with(code, error, message)
      halt code, {"Content-Type" => "application/json"},
        JSON.generate(error: {code: error, message: message, request_id: request_id})
    end

    def read_json
      fail_with 413, "body_too_large", "request body exceeds #{MAX_BODY_BYTES} bytes" if request.content_length.to_i > MAX_BODY_BYTES
      raw = request.body.read(MAX_BODY_BYTES + 1).to_s
      fail_with 413, "body_too_large", "request body exceeds #{MAX_BODY_BYTES} bytes" if raw.bytesize > MAX_BODY_BYTES
      JSON.parse(raw)
    rescue JSON::ParserError
      fail_with 400, "invalid_json", "request body is not valid JSON"
    end

    def engine_call(method, params = nil)
      settings.engine.call(method, params, request_id: request_id)
    rescue EngineClient::RemoteError => e
      fail_with ENGINE_STATUS.fetch(e.code, 502), e.code, e.message
    rescue EngineClient::Unavailable
      fail_with 503, "engine_unavailable", "media engine is not reachable"
    end

    def scene_row(id)
      fail_with 400, "invalid_id", "scene id must match #{ID.source}" unless id.match?(ID)
      settings.db[:scenes].where(id: id).first
    end

    def scene_json(row)
      {id: row[:id], revision: row[:revision], document: JSON.parse(row[:document])}
    end
  end

  before do
    headers "Cache-Control" => "no-store", "X-Content-Type-Options" => "nosniff",
      "X-Request-Id" => request_id,
      "Content-Security-Policy" => "default-src 'self'; style-src 'self' 'unsafe-inline'; frame-ancestors 'none'; base-uri 'none'"
    @lan_request = !ALLOWED_HOSTS.include?(request.host)
    if @lan_request
      # Non-loopback Host values are accepted only over TLS and only for the
      # names/addresses in the device certificate (DNS-rebinding defence).
      fail_with 403, "forbidden_host", "LAN access requires TLS to a certified device name" unless request.ssl? && settings.lan&.lan_host?(request.host)
      headers "Strict-Transport-Security" => "max-age=31536000"
    end
    origin = request.env["HTTP_ORIGIN"]
    fail_with 403, "forbidden_origin", "cross-origin requests are not allowed" if origin && origin != "#{request.scheme}://#{request.host_with_port}"
  end

  before "/api/*" do
    content_type :json
    token = settings.token
    env_token = token.is_a?(String) && token.bytesize >= 32
    unless env_token || settings.lan&.configured?
      fail_with 503, "not_configured", "T3BOX_CONTROL_TOKEN must be at least 32 bytes"
    end
    header = request.env.fetch("HTTP_AUTHORIZATION", "")
    supplied = header.delete_prefix("Bearer ")
    bearer = header.start_with?("Bearer ")
    # The bootstrap token is loopback-only; the LAN accepts only tokens the
    # owner enrolled over BLE (revoked by factory reset).
    local = !@lan_request && bearer && env_token && supplied.bytesize == token.bytesize &&
      OpenSSL.fixed_length_secure_compare(supplied, token)
    authorized = local || (bearer && settings.lan&.issued_token?(supplied))
    fail_with 401, "unauthorized", "missing or invalid bearer token" unless authorized
  end

  get "/api/v1/capabilities" do
    json_reply({scene_drafts: true, program_apply: false, phone_camera: false, ble: false})
  end

  get "/api/v1/status" do
    result = engine_call("engine.status")
    restarted = Control.observe_engine_instance(result["instance_id"])
    json_reply(result.merge("engine_restarted" => restarted))
  end

  # Server-Sent Events (text/event-stream). First event is `snapshot`
  # ({engine, scenes}); then engine.connected / engine.unavailable /
  # engine.restarted / stream.state_changed / program.changed /
  # scene.updated / scene.deleted, plus comment keepalives. Bounded to
  # EventHub::MAX_CLIENTS concurrent streams (429 beyond).
  get "/api/v1/events" do
    hub = settings.events
    settings.poller.start
    snapshot = {engine: settings.poller.last, scenes: settings.db[:scenes].order(:id).select(:id, :revision).all}
    sub = hub.subscribe
    fail_with 429, "too_many_streams", "at most #{EventHub::MAX_CLIENTS} event streams" unless sub
    headers "Content-Type" => "text/event-stream", "X-Accel-Buffering" => "no"
    halt 200, EventHub::Stream.new(hub, sub, snapshot)
  end

  get "/api/v1/engine/capabilities" do
    json_reply(engine_call("engine.capabilities"))
  end

  post "/api/v1/stream/start" do
    json_reply(engine_call("stream.start"), 202)
  end

  post "/api/v1/stream/stop" do
    json_reply(engine_call("stream.stop"))
  end

  get "/api/v1/scenes" do
    json_reply(settings.db[:scenes].order(:id).all.map { |row| scene_json(row) })
  end

  get "/api/v1/scenes/:id" do
    row = scene_row(params[:id])
    fail_with 404, "not_found", "scene does not exist" unless row
    json_reply(scene_json(row))
  end

  get "/api/v1/scenes/:id/revisions" do
    row = scene_row(params[:id])
    fail_with 404, "not_found", "scene does not exist" unless row
    revisions = settings.db[:scene_revisions].where(scene_id: row[:id]).reverse(:revision).all
    json_reply(revisions.map { |r| {revision: r[:revision], created_at_unix: r[:created_at_unix], document: JSON.parse(r[:document])} })
  end

  delete "/api/v1/scenes/:id" do
    row = scene_row(params[:id])
    fail_with 404, "not_found", "scene does not exist" unless row
    expected = Integer(params["revision"], exception: false)
    fail_with 422, "invalid_request", "revision query parameter required" unless expected
    deleted = settings.db[:scenes].where(id: row[:id], revision: expected).delete
    fail_with 409, "revision_conflict", "scene revision is #{row[:revision]}" if deleted.zero?
    settings.events.publish("scene.deleted", {id: row[:id]})
    status 204
    ""
  end

  # Draft save with optimistic concurrency. `revision` is the revision the
  # client last saw (0 to create). Saving a draft never changes Program.
  put "/api/v1/scenes/:id" do
    id = params[:id]
    scene_row(id)
    input = read_json
    unless input.is_a?(Hash) && input.keys.sort == %w[document revision] &&
        input["revision"].is_a?(Integer) && input["revision"] >= 0
      fail_with 422, "invalid_request", "body must be {revision, document}"
    end
    if (violation = SceneValidator.validate(input["document"]))
      fail_with 422, "invalid_scene", violation.join(": ")
    end
    revision = input["revision"]
    document = JSON.generate(input["document"])
    db = settings.db
    saved = db.transaction(mode: :immediate) do
      row = db[:scenes].where(id: id).first
      if (row && row[:revision] != revision) || (!row && revision != 0)
        false
      else
        if row
          db[:scenes].where(id: id).update(revision: revision + 1, document: document)
        else
          db[:scenes].insert(id: id, revision: 1, document: document)
        end
        db[:scene_revisions].insert(scene_id: id, revision: revision + 1, document: document, created_at_unix: Time.now.to_i)
        cutoff = revision + 1 - MAX_REVISIONS_KEPT
        db[:scene_revisions].where(scene_id: id).where(Sequel[:revision] <= cutoff).delete
        true
      end
    end
    fail_with 409, "revision_conflict", "scene was modified; reload and retry" unless saved
    settings.events.publish("scene.updated", {id: id, revision: revision + 1})
    json_reply({id: id, revision: revision + 1, document: input["document"]})
  end

  # Request Program transition to a stored draft revision. The engine
  # validates independently and answers only after the media thread
  # acknowledged frame-boundary application (first_frame_index). Without the
  # compositor (501), without a running stream (409 program_inactive) or on
  # timeout (503) Program is unchanged.
  post "/api/v1/program/transition" do
    input = read_json
    unless input.is_a?(Hash) && input.keys.sort == %w[revision scene_id] &&
        input["scene_id"].is_a?(String) && input["revision"].is_a?(Integer)
      fail_with 422, "invalid_request", "body must be {scene_id, revision}"
    end
    row = scene_row(input["scene_id"])
    fail_with 404, "not_found", "scene does not exist" unless row
    fail_with 409, "revision_conflict", "scene revision is #{row[:revision]}" unless row[:revision] == input["revision"]
    result = engine_call("scene.apply", {scene: JSON.parse(row[:document]), scene_id: row[:id], revision: row[:revision]})
    # Only reachable once the engine reports an acknowledged application.
    settings.events.publish("program.changed", {scene_id: row[:id], revision: row[:revision]})
    json_reply({scene_id: row[:id], revision: row[:revision], engine: result})
  end

  get "/" do
    index = File.join(settings.public_folder, "index.html")
    fail_with 404, "not_found", "studio is not built" unless File.file?(index)
    send_file index
  end

  not_found do
    content_type :json
    JSON.generate(error: {code: "not_found", message: "no such route", request_id: request_id})
  end

  error do
    content_type :json
    status 500
    JSON.generate(error: {code: "internal_error", message: "unexpected server error", request_id: request_id})
  end
end
