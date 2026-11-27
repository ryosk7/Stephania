# frozen_string_literal: true

# Draft scene v1 validation for the management plane. Mirrors src/scene.rs;
# both are exercised against contracts/fixtures/scenes so they cannot drift.
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

  module_function

  def validate(doc)
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
