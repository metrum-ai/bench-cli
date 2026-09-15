// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

package vision

import "encoding/json"

// ImageTokensEstimate is the fixed token cost for each image_url part.
const ImageTokensEstimate = 256

// Part is one multimodal content part (text or image_url).
type Part struct {
	Type     string    `json:"type"`
	Text     string    `json:"text,omitempty"`
	ImageURL *ImageURL `json:"image_url,omitempty"`
}

// ImageURL holds an image reference in OpenAI chat format.
type ImageURL struct {
	URL    string `json:"url"`
	Detail string `json:"detail,omitempty"`
}

// Content unmarshals from either a string or an array of parts.
type Content []Part

// UnmarshalJSON accepts string or []Part.
func (c *Content) UnmarshalJSON(data []byte) error {
	if len(data) == 0 || string(data) == "null" {
		*c = nil
		return nil
	}
	if data[0] == '"' {
		var s string
		if err := json.Unmarshal(data, &s); err != nil {
			return err
		}
		*c = []Part{{Type: "text", Text: s}}
		return nil
	}
	var parts []Part
	if err := json.Unmarshal(data, &parts); err != nil {
		return err
	}
	*c = parts
	return nil
}

// TextLen returns total rune/byte length of text parts (bytes, matching len/4 tokenizer).
func (c Content) TextLen() int {
	n := 0
	for _, p := range c {
		if p.Type == "text" || p.Type == "" {
			n += len(p.Text)
		}
	}
	return n
}

// PromptTokens estimates prompt tokens: len(text)/4 plus ImageTokensEstimate per image.
func PromptTokens(messages []struct {
	Role    string
	Content Content
}) int {
	n := 0
	for _, m := range messages {
		n += m.Content.TextLen() / 4
		for _, p := range m.Content {
			if p.Type == "image_url" {
				n += ImageTokensEstimate
			}
		}
	}
	if n == 0 {
		return 1
	}
	return n
}

// PromptTokensFromParts sums tokens for a list of Contents.
func PromptTokensFromParts(contents []Content) int {
	n := 0
	for _, c := range contents {
		n += c.TextLen() / 4
		for _, p := range c {
			if p.Type == "image_url" {
				n += ImageTokensEstimate
			}
		}
	}
	if n == 0 {
		return 1
	}
	return n
}
