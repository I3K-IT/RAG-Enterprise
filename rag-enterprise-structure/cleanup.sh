#!/bin/bash

set -e

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m'

echo -e "${BLUE}╔════════════════════════════════════════════════════════╗${NC}"
echo -e "${BLUE}║   RAG Enterprise - Complete Cleanup Script            ║${NC}"
echo -e "${BLUE}║   ⚠️  This will remove EVERYTHING installed by setup  ║${NC}"
echo -e "${BLUE}╚════════════════════════════════════════════════════════╝${NC}\n"

echo -e "${YELLOW}This script will remove:${NC}"
echo -e "  - All Docker containers, volumes, and images"
echo -e "  - Docker installation (docker-ce, docker-compose, etc.)"
echo -e "  - NVIDIA Container Toolkit"
echo -e "  - Cache directories (~/.ollama, ~/.cache/huggingface, ~/.paddleocr)"
echo -e "  - Docker configurations and repositories"
echo -e "  - User from docker group\n"

read -p "$(echo -e ${RED}Are you sure you want to continue? [y/N]: ${NC})" -n 1 -r
echo
if [[ ! $REPLY =~ ^[Yy]$ ]]; then
    echo -e "${GREEN}Cleanup cancelled.${NC}"
    exit 0
fi

# ============================================================================
# STEP 1: Stop and Remove Docker Containers
# ============================================================================

echo -e "\n${YELLOW}[1/8] Stopping and removing Docker containers...${NC}"

if command -v docker &> /dev/null; then
    # Stop all running containers
    if [ "$(sudo docker ps -q)" ]; then
        echo "Stopping running containers..."
        sudo docker stop $(sudo docker ps -q) 2>/dev/null || true
    fi

    # Stop docker-compose services if in project directory
    if [ -f "docker-compose.yml" ]; then
        echo "Stopping docker-compose services..."
        sudo docker compose down -v 2>/dev/null || true
    fi

    # Remove all containers
    if [ "$(sudo docker ps -aq)" ]; then
        echo "Removing all containers..."
        sudo docker rm -f $(sudo docker ps -aq) 2>/dev/null || true
    fi

    echo -e "${GREEN}✓ Containers stopped and removed${NC}"
else
    echo -e "${YELLOW}⚠ Docker not found, skipping container cleanup${NC}"
fi

# ============================================================================
# STEP 2: Remove Docker Volumes
# ============================================================================

echo -e "\n${YELLOW}[2/8] Removing Docker volumes...${NC}"

if command -v docker &> /dev/null; then
    if [ "$(sudo docker volume ls -q)" ]; then
        echo "Removing all volumes..."
        sudo docker volume rm -f $(sudo docker volume ls -q) 2>/dev/null || true
    fi
    echo -e "${GREEN}✓ Volumes removed${NC}"
else
    echo -e "${YELLOW}⚠ Docker not found, skipping volume cleanup${NC}"
fi

# ============================================================================
# STEP 3: Remove Docker Images
# ============================================================================

echo -e "\n${YELLOW}[3/8] Removing Docker images...${NC}"

if command -v docker &> /dev/null; then
    if [ "$(sudo docker images -q)" ]; then
        echo "Removing all images..."
        sudo docker rmi -f $(sudo docker images -q) 2>/dev/null || true
    fi
    echo -e "${GREEN}✓ Images removed${NC}"
else
    echo -e "${YELLOW}⚠ Docker not found, skipping image cleanup${NC}"
fi

# ============================================================================
# STEP 4: Remove Docker Networks
# ============================================================================

echo -e "\n${YELLOW}[4/8] Removing Docker networks...${NC}"

if command -v docker &> /dev/null; then
    # Get custom networks (exclude default ones)
    CUSTOM_NETWORKS=$(sudo docker network ls --filter type=custom -q 2>/dev/null || true)
    if [ ! -z "$CUSTOM_NETWORKS" ]; then
        echo "Removing custom networks..."
        sudo docker network rm $CUSTOM_NETWORKS 2>/dev/null || true
    fi
    echo -e "${GREEN}✓ Networks removed${NC}"
else
    echo -e "${YELLOW}⚠ Docker not found, skipping network cleanup${NC}"
fi

# ============================================================================
# STEP 5: Uninstall Docker and NVIDIA Toolkit
# ============================================================================

echo -e "\n${YELLOW}[5/8] Uninstalling Docker and NVIDIA Container Toolkit...${NC}"

# Stop Docker service
echo "Stopping Docker service..."
sudo systemctl stop docker 2>/dev/null || true
sudo systemctl stop docker.socket 2>/dev/null || true

# Remove Docker packages
echo "Removing Docker packages..."
sudo apt-get purge -y docker-ce docker-ce-cli containerd.io docker-buildx-plugin docker-compose-plugin 2>/dev/null || true
sudo apt-get purge -y docker-compose 2>/dev/null || true

# Remove NVIDIA Container Toolkit
echo "Removing NVIDIA Container Toolkit..."
sudo apt-get purge -y nvidia-container-toolkit nvidia-docker2 2>/dev/null || true

# Clean up unused packages
sudo apt-get autoremove -y
sudo apt-get autoclean

echo -e "${GREEN}✓ Docker and NVIDIA toolkit uninstalled${NC}"

# ============================================================================
# STEP 6: Remove Docker Directories and Configurations
# ============================================================================

echo -e "\n${YELLOW}[6/8] Removing Docker directories and configurations...${NC}"

echo "Removing Docker system directories..."
sudo rm -rf /var/lib/docker
sudo rm -rf /var/lib/containerd
sudo rm -rf /etc/docker
sudo rm -rf /var/run/docker
sudo rm -rf /var/run/docker.sock
sudo rm -rf /usr/local/bin/docker-compose

echo "Removing Docker repository configuration..."
sudo rm -rf /etc/apt/sources.list.d/docker.list
sudo rm -rf /etc/apt/keyrings/docker.gpg

echo "Removing Docker binaries..."
sudo rm -f /usr/bin/docker*
sudo rm -f /usr/local/bin/docker*

echo -e "${GREEN}✓ Docker directories and configs removed${NC}"

# ============================================================================
# STEP 7: Remove Cache Directories
# ============================================================================

echo -e "\n${YELLOW}[7/8] Removing cache directories...${NC}"

echo "Removing Ollama cache..."
rm -rf ~/.ollama

echo "Removing HuggingFace cache..."
rm -rf ~/.cache/huggingface

echo "Removing PaddleOCR cache..."
rm -rf ~/.paddleocr

echo -e "${GREEN}✓ Cache directories removed${NC}"

# ============================================================================
# STEP 8: Remove User from Docker Group
# ============================================================================

echo -e "\n${YELLOW}[8/8] Removing user from Docker group...${NC}"

sudo deluser $USER docker 2>/dev/null || true
sudo groupdel docker 2>/dev/null || true

echo -e "${GREEN}✓ User removed from docker group${NC}"

# ============================================================================
# FINAL CLEANUP
# ============================================================================

echo -e "\n${YELLOW}Performing final cleanup...${NC}"

# Update package database
sudo apt-get update -qq

echo -e "\n${GREEN}╔════════════════════════════════════════════════════════╗${NC}"
echo -e "${GREEN}║   ✓ Complete Cleanup Finished!                        ║${NC}"
echo -e "${GREEN}╚════════════════════════════════════════════════════════╝${NC}\n"

echo -e "${BLUE}What was removed:${NC}"
echo -e "  ${GREEN}✓${NC} All Docker containers, volumes, and images"
echo -e "  ${GREEN}✓${NC} Docker CE and Docker Compose"
echo -e "  ${GREEN}✓${NC} NVIDIA Container Toolkit"
echo -e "  ${GREEN}✓${NC} Cache directories"
echo -e "  ${GREEN}✓${NC} Docker configurations"
echo -e "  ${GREEN}✓${NC} User permissions\n"

echo -e "${CYAN}Next steps:${NC}"
echo -e "  1. Logout and login again (to apply group changes)"
echo -e "  2. Run: ${YELLOW}./setup.sh standard${NC}"
echo -e "  3. Follow the setup wizard\n"

echo -e "${YELLOW}⚠ Remember to logout/login before running setup.sh!${NC}\n"
