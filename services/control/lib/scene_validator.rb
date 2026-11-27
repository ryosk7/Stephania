# frozen_string_literal: true

# Scene validation (contract versions 1 and 2, docs/SCENE.md) for the
# management plane. Mirrors src/scene.rs; Rust, Ruby and TypeScript are all
# exercised against contracts/fixtures/scenes so they cannot drift.
# Returns nil when valid, otherwise [path, reason] for the first violation.
module SceneValidator
  CANVAS_WIDTH = 1920
  CANVAS_HEIGHT = 1080
  MAX_LAYERS = 16
  MAX_NAME_CHARS = 100
  SOURCES_V1 = %w[hdmi phone:rear phone:front].freeze
  ROOT_KEYS = %w[layers name version].freeze
  LAYER_KEYS = %w[height id source visible width x y].freeze
  ID = /\A[a-zA-Z0-9_-]{1,64}\z/

  FITS = %w[stretch contain cover].freeze
  TYPES = %w[video image color].freeze
  COMMON_V2 = %w[id type x y width height visible].freeze
  BY_TYPE = {
    "video" => [%w[source], %w[opacity rotation fit crop]],
    "image" => [%w[asset], %w[opacity rotation fit template]],
    "color" => [%w[color], %w[opacity rotation]]
  }.freeze
  HEX_COLOR = /\A#\h{6}\z/
  ASSET = /\A[0-9a-f]{16}\z/
  TEMPLATE_ID = /\A[a-z0-9-]{1,32}\z/
  TEMPLATE_KEY = /\A[a-z][a-z0-9_.]{0,31}\z/
  MAX_CROP_SUM = 9000

  module_function

  def validate(doc)
    return validate_v2(doc) if doc.is_a?(Hash) && doc["version"] == 2 && doc["version"].is_a?(Integer)

    validate_v1(doc)
  end

  def keys_ok?(hash, required, optional)
    required.all? { |k| hash.key?(k) } && (hash.keys - required - optional).empty?
  end

  def int?(v) = v.is_a?(Integer)

  def template_ok?(t)
    return false unless t.is_a?(Hash) && t.keys.sort == %w[id vars] && t["id"].is_a?(String) && t["id"].match?(TEMPLATE_ID)

    vars = t["vars"]
    vars.is_a?(Hash) && vars.size <= 16 &&
      vars.all? { |k, v| k.match?(TEMPLATE_KEY) && v.is_a?(String) && v.length <= 200 }
  end

  def validate_v2(doc)
    return ["", "unexpected_or_missing_property"] unless keys_ok?(doc, %w[version name layers], %w[canvas])

    name = doc["name"]
    return ["/name", "invalid_name"] unless name.is_a?(String) && name.length.between?(1, MAX_NAME_CHARS)
    if doc.key?("canvas")
      c = doc["canvas"]
      return ["/canvas", "invalid_canvas"] unless c.is_a?(Hash) && c.keys == ["background"] && c["background"].is_a?(String) && c["background"].match?(HEX_COLOR)
    end
    layers = doc["layers"]
    return ["/layers", "not_array"] unless layers.is_a?(Array)
    return ["/layers", "too_many_layers"] if layers.length > MAX_LAYERS

    seen = {}
    layers.each_with_index do |layer, index|
      path = "/layers/#{index}"
      return [path, "not_object"] unless layer.is_a?(Hash)

      kind = layer["type"]
      return ["#{path}/type", "unknown_type"] unless TYPES.include?(kind)

      required, optional = BY_TYPE.fetch(kind)
      return [path, "unexpected_or_missing_property"] unless keys_ok?(layer, COMMON_V2 + required, optional)

      id = layer["id"]
      return ["#{path}/id", "invalid_id"] unless id.is_a?(String) && id.match?(ID)
      return ["#{path}/id", "duplicate_id"] if seen[id]

      seen[id] = true
      return ["#{path}/visible", "not_boolean"] unless [true, false].include?(layer["visible"])

      %w[x y width height].each do |key|
        return ["#{path}/#{key}", "not_integer"] unless int?(layer[key])
      end
      x, y, w, h = layer.values_at("x", "y", "width", "height")
      return [path, "negative_or_empty_geometry"] if x.negative? || y.negative? || w <= 0 || h <= 0
      return [path, "outside_canvas"] if x + w > CANVAS_WIDTH || y + h > CANVAS_HEIGHT
      if layer.key?("opacity")
        return ["#{path}/opacity", "not_integer"] unless int?(layer["opacity"])
        return ["#{path}/opacity", "invalid_opacity"] unless layer["opacity"].between?(0, 100)
      end
      if layer.key?("rotation")
        return ["#{path}/rotation", "not_integer"] unless int?(layer["rotation"])
        return ["#{path}/rotation", "invalid_rotation"] unless layer["rotation"].between?(-180, 180)
      end
      return ["#{path}/fit", "invalid_fit"] if layer.key?("fit") && !FITS.include?(layer["fit"])

      case kind
      when "video"
        return ["#{path}/source", "unknown_source"] unless SOURCES_V1.include?(layer["source"])
        if layer.key?("crop")
          c = layer["crop"]
          ok = c.is_a?(Hash) && c.keys.sort == %w[bottom left right top] && c.values.all? { |v| int?(v) && v.between?(0, MAX_CROP_SUM) } &&
            c["left"] + c["right"] <= MAX_CROP_SUM && c["top"] + c["bottom"] <= MAX_CROP_SUM
          return ["#{path}/crop", "invalid_crop"] unless ok
        end
      when "image"
        return ["#{path}/asset", "invalid_asset"] unless layer["asset"].is_a?(String) && layer["asset"].match?(ASSET)
        return ["#{path}/template", "invalid_template"] if layer.key?("template") && !template_ok?(layer["template"])
      else
        return ["#{path}/color", "invalid_color"] unless layer["color"].is_a?(String) && layer["color"].match?(HEX_COLOR)
      end
    end
    nil
  end

  def validate_v1(doc)
    return ["", "not_object"] unless doc.is_a?(Hash)
    return ["", "unexpected_or_missing_property"] unless doc.keys.sort == ROOT_KEYS
    return ["/version", "unsupported_version"] unless doc["version"] == 1 && doc["version"].is_a?(Integer)

    name = doc["name"]
    return ["/name", "invalid_name"] unless name.is_a?(String) && name.length.between?(1, MAX_NAME_CHARS)

    layers = doc["layers"]
    return ["/layers", "not_array"] unless layers.is_a?(Array)
    return ["/layers", "too_many_layers"] if layers.length > MAX_LAYERS

    seen = {}
    layers.each_with_index do |layer, index|
      path = "/layers/#{index}"
      return [path, "not_object"] unless layer.is_a?(Hash)
      return [path, "unexpected_or_missing_property"] unless layer.keys.sort == LAYER_KEYS

      id = layer["id"]
      return ["#{path}/id", "invalid_id"] unless id.is_a?(String) && id.match?(ID)
      return ["#{path}/id", "duplicate_id"] if seen[id]

      seen[id] = true
      return ["#{path}/source", "unknown_source"] unless SOURCES_V1.include?(layer["source"])
      return ["#{path}/visible", "not_boolean"] unless [true, false].include?(layer["visible"])

      %w[x y width height].each do |key|
        return ["#{path}/#{key}", "not_integer"] unless layer[key].is_a?(Integer)
      end
      x, y, w, h = layer.values_at("x", "y", "width", "height")
      return [path, "negative_or_empty_geometry"] if x.negative? || y.negative? || w <= 0 || h <= 0
      return [path, "outside_canvas"] if x + w > CANVAS_WIDTH || y + h > CANVAS_HEIGHT
    end
    nil
  end

  def valid?(doc) = validate(doc).nil?
end
