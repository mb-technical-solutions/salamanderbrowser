#!/bin/bash
set -e

# Colors for output
BLUE='\033[0;34m'
NC='\033[0m' # No Color

echo -e "${BLUE}========================================${NC}"
echo -e "${BLUE}  Salamander Browser Development Environment${NC}"
echo -e "${BLUE}========================================${NC}"
echo ""
echo -e "${BLUE}Frontend: http://localhost:12341${NC}"
echo ""

pnpm tauri dev
