#!/usr/bin/env bash
# Creates the v0.1.0 GitHub release for bloom-petal-gasless with all artifacts.
# Usage: GH_TOKEN=<token> bash scripts/create-release.sh
set -euo pipefail

REPO="bloom-directory/bloom-petal-gasless"
TAG="v0.1.0"
COMMIT="727765fdde5ce3111a4cf65123c72de68649a6e2"
STAGING="/tmp/gasless-release"

if [ -z "${GH_TOKEN:-}" ]; then
  echo "Error: GH_TOKEN environment variable is required"
  echo "Create a token at https://github.com/settings/tokens (needs repo scope)"
  exit 1
fi

echo "Creating release $TAG..."

# Create the release
RELEASE_JSON=$(curl -s -X POST \
  -H "Authorization: token $GH_TOKEN" \
  -H "Content-Type: application/json" \
  "https://api.github.com/repos/$REPO/releases" \
  -d "$(cat <<EOF
{
  "tag_name": "$TAG",
  "target_commitish": "$COMMIT",
  "name": "$TAG",
  "body": "## gasless petal v0.1.0\n\nGeneric Relay EIP-3009 permit flow for same-chain and cross-chain transfers/swaps.\n\n**5 routes** — \`transactions/<wallet>/<id>.json\`, \`status.json\`, \`chains/\`, \`README.md\`, \`AGENTS.md\`\n\n**Supported chains**: ethereum, base, arbitrum, optimism, polygon, avalanche, Hyperliquid\n\n**Package hash**: \`d597058a\`",
  "draft": false,
  "prerelease": false
}
EOF
)")

RELEASE_ID=$(echo "$RELEASE_JSON" | python3 -c "import sys,json; print(json.load(sys.stdin)['id'])")
UPLOAD_URL="https://uploads.github.com/repos/$REPO/releases/$RELEASE_ID/assets"

echo "Release ID: $RELEASE_ID"
echo "Uploading assets..."

for file in petal-release.json SHA256SUMS gasless-v0.1.0.petal.tar.gz; do
  echo "  Uploading $file..."
  if [[ "$file" == *.tar.gz ]]; then
    CONTENT_TYPE="application/gzip"
  else
    CONTENT_TYPE="text/plain"
  fi
  curl -s -X POST \
    -H "Authorization: token $GH_TOKEN" \
    -H "Content-Type: $CONTENT_TYPE" \
    --data-binary "@$STAGING/$file" \
    "$UPLOAD_URL?name=$file" | python3 -c "import sys,json; d=json.load(sys.stdin); print(f'    ✓ {d[\"name\"]} ({d[\"size\"]} bytes)')"
done

echo ""
echo "Release $TAG published: https://github.com/$REPO/releases/tag/$TAG"
