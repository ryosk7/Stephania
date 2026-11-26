require "sinatra/base"
require "json"
require "sequel"
require "net/http"
require "openssl"

class Control < Sinatra::Base
  set :bind, "127.0.0.1"
  set :public_folder, File.expand_path("../../apps/studio/dist", __dir__)
  set :show_exceptions, false
  set :raise_errors, false
  set :logging, false
  set :db, nil
  set :token, nil
  set :engine, nil

  def self.configure_storage(path)
    db = Sequel.sqlite(path, max_connections: 1, timeout: 1000)
    db.create_table? :scenes do
      String :id, primary_key: true
      Integer :revision, null: false
      String :document, text: true, null: false
    end
    set :db, db
  end

  configure do
    token = ENV["T3BOX_CONTROL_TOKEN"]
    set :token, token
    configure_storage(ENV.fetch("T3BOX_CONTROL_DB", ":memory:"))
  end

  helpers do
    def json_reply(value, code = 200)
      status code
      JSON.generate(value)
    end

    def read_json
      halt 413, JSON.generate(error: "body_too_large") if request.content_length.to_i > 65536
      raw = request.body.read(65537)
      halt 413, JSON.generate(error: "body_too_large") if raw.bytesize > 65536
      JSON.parse(raw)
    rescue JSON::ParserError
      halt 400, JSON.generate(error: "invalid_json")
    end

    def valid_scene?(doc)
      return false unless doc.is_a?(Hash) && doc.keys.sort == %w[layers name version]
      return false unless doc["version"] == 1 && doc["name"].is_a?(String) && doc["name"].length.between?(1, 100)
      layers = doc["layers"]
      return false unless layers.is_a?(Array) && layers.length <= 16
      ids = []
      layers.all? do |l|
        next false unless l.is_a?(Hash) && l.keys.sort == %w[height id source visible width x y]
        next false unless l["id"].is_a?(String) && l["id"].match?(/\A[a-zA-Z0-9_-]{1,64}\z/) && !ids.include?(l["id"])
        ids << l["id"]
        next false unless %w[hdmi phone:rear phone:front].include?(l["source"]) && [true,false].include?(l["visible"])
        next false unless %w[x y width height].all? { |k| l[k].is_a?(Integer) }
        l["x"] >= 0 && l["y"] >= 0 && l["width"] > 0 && l["height"] > 0 &&
          l["x"] + l["width"] <= 1920 && l["y"] + l["height"] <= 1080
      end
    end

    def engine_call(method, path)
      return settings.engine.call(method, path) if settings.engine
      http = Net::HTTP.new("127.0.0.1", 8787, nil)
      http.open_timeout = 1
      http.read_timeout = 2
      http.write_timeout = 2
      response = http.request(method == "GET" ? Net::HTTP::Get.new(path) : Net::HTTP::Post.new(path))
      [response.code.to_i, JSON.parse(response.body)]
    rescue IOError, SystemCallError, Timeout::Error, JSON::ParserError
      [503, {error: "engine_unavailable"}]
    end
  end

  before do
    headers "Cache-Control" => "no-store", "X-Content-Type-Options" => "nosniff",
      "Content-Security-Policy" => "default-src 'self'; style-src 'self' 'unsafe-inline'; frame-ancestors 'none'; base-uri 'none'"
    halt 403 unless ["localhost", "127.0.0.1"].include?(request.host)
  end

  before "/api/*" do
    content_type :json
    token = settings.token
    halt 503, JSON.generate(error: "not_configured") unless token && token.bytesize >= 32
    supplied = request.env.fetch("HTTP_AUTHORIZATION", "").delete_prefix("Bearer ")
    halt 401, JSON.generate(error: "unauthorized") unless request.env.fetch("HTTP_AUTHORIZATION", "").start_with?("Bearer ") &&
      supplied.bytesize == token.bytesize && OpenSSL.fixed_length_secure_compare(supplied, token)
  end

  get "/api/v1/capabilities" do
    json_reply({scene_drafts: true, program_apply: false, phone_camera: false, ble: false})
  end
  get "/api/v1/status" do
    code, data = engine_call("GET", "/status")
    json_reply(data, code)
  end
  post "/api/v1/stream/start" do
    code, data = engine_call("POST", "/stream/start")
    json_reply(data, code)
  end
  post "/api/v1/stream/stop" do
    code, data = engine_call("POST", "/stream/stop")
    json_reply(data, code)
  end
  get "/api/v1/scenes" do
    json_reply(settings.db[:scenes].order(:id).all.map { |r| {id: r[:id], revision: r[:revision], document: JSON.parse(r[:document])} })
  end
  put "/api/v1/scenes/:id" do
    id = params[:id]
    halt 400, JSON.generate(error: "invalid_id") unless id.match?(/\A[a-zA-Z0-9_-]{1,64}\z/)
    input = read_json
    halt 422, JSON.generate(error: "invalid_scene") unless input.is_a?(Hash) && input["revision"].is_a?(Integer) &&
      input["revision"] >= 0 && valid_scene?(input["document"])
    revision = input["revision"]
    doc = JSON.generate(input["document"])
    result = settings.db.transaction(mode: :immediate) do
      row = settings.db[:scenes].where(id: id).first
      if (row && row[:revision] != revision) || (!row && revision != 0)
        false
      elsif row
        settings.db[:scenes].where(id: id).update(revision: revision + 1, document: doc)
        true
      else
        settings.db[:scenes].insert(id: id, revision: 1, document: doc)
        true
      end
    end
    halt 409, JSON.generate(error: "revision_conflict") unless result
    json_reply({id: id, revision: revision + 1, document: input["document"]})
  end
  post "/api/v1/program/transition" do
    json_reply({error: "compositor_not_implemented"}, 501)
  end
  get "/" do
    send_file File.join(settings.public_folder, "index.html")
  end
end
