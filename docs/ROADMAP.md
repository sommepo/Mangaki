# Roadmap

Items here are intentionally deferred and are not represented as implemented acceptance scope.

## Configurable OCR server

- Add an explicit Custom OCR server provider alongside bundled ML Kit and consented Google Lens.
- Support a URL, optional Android-Keystore-backed credentials, connection/capability testing, and clear upload disclosure.
- Define a versioned response contract for normalized text-block geometry, orientation, confidence, engine, and model version.
- Include provider/model identity in cache keys and offer targeted reprocessing after a provider or model change.
- Provide a separately deployable reference service for manga-specific recognition, initially evaluating `manga-ocr` with an appropriate detector and block reconstruction stage.
- Never silently fall back from private on-device processing to a network provider.

## Exact experimental word geometry

- Preserve line/element/symbol geometry from OCR providers through block merging and cache serialization.
- Map dictionary match spans to those source glyph boxes instead of measuring transparent replacement-font text.
- Reprocess older cached pages when the geometry schema changes; a stronger recognition model alone cannot eliminate replacement-font alignment error.
