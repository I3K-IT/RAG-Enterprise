#!/bin/bash

set -e

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m'

# Allow resuming from a specific step
START_STEP="${2:-0}"  # Default: start from step 0

if [ ! -z "$2" ]; then
    echo -e "${YELLOW}Resuming from step $START_STEP${NC}"
fi

# ============================================================================
# CONFIGURATION
# ============================================================================

PROFILE="${1:-standard}"  # Default: standard

# Validate profile
if [[ ! "$PROFILE" =~ ^(minimal|standard|advanced)$ ]]; then
    echo -e "${RED}❌ Invalid profile: $PROFILE${NC}"
    echo "Usage: ./setup.sh [minimal|standard|advanced]"
    exit 1
fi

# ============================================================================
# PROFILE DEFINITIONS
# ============================================================================

declare -A PROFILES_CPU=(
    [minimal]="8"
    [standard]="16"
    [advanced]="32"
)

declare -A PROFILES_RAM=(
    [minimal]="16"
    [standard]="32"
    [advanced]="128"
)

declare -A PROFILES_GPU=(
    [minimal]="8-12GB"
    [standard]="12-16GB"
    [advanced]="16-24GB"
)

declare -A PROFILES_EMBEDDING=(
    [minimal]="all-mpnet-base-v2"
    [standard]="BAAI/bge-m3"
    [advanced]="instructor-large"
)

declare -A PROFILES_LLM=(
    [minimal]="mistral:7b-instruct-q4_K_M"
    [standard]="qwen2.5:14b-instruct-q4_K_M"
    [advanced]="qwen2.5:32b-instruct-q4_K_M"
)

declare -A PROFILES_SPACE=(
    [minimal]="20GB"
    [standard]="50GB"
    [advanced]="100GB"
)

# ============================================================================
# HELPER FUNCTIONS
# ============================================================================

print_header() {
    echo -e "\n${BLUE}╔════════════════════════════════════════════════════════╗${NC}"
    echo -e "${BLUE}║   RAG Enterprise - Setup Wizard                        ║${NC}"
    echo -e "${BLUE}║   Profile: ${CYAN}$(echo $PROFILE | tr '[:lower:]' '[:upper:]')$(printf '%*s' $((38 - ${#PROFILE})) '')${BLUE}║${NC}"
    echo -e "${BLUE}╚════════════════════════════════════════════════════════╝${NC}\n"
}

print_profile_info() {
    echo -e "${CYAN}📊 Profile Requirements:${NC}"
    echo -e "  CPU Cores:    ${YELLOW}${PROFILES_CPU[$PROFILE]}+${NC}"
    echo -e "  RAM:          ${YELLOW}${PROFILES_RAM[$PROFILE]}GB${NC}"
    echo -e "  GPU VRAM:     ${YELLOW}${PROFILES_GPU[$PROFILE]}${NC}"
    echo -e "  Storage:      ${YELLOW}${PROFILES_SPACE[$PROFILE]}${NC}"
    echo -e "  Embedding:    ${YELLOW}${PROFILES_EMBEDDING[$PROFILE]}${NC}"
    echo -e "  LLM Model:    ${YELLOW}${PROFILES_LLM[$PROFILE]}${NC}\n"
}

check_command() {
    if ! command -v $1 &> /dev/null; then
        echo -e "${RED}✗ $1 not found${NC}"
        return 1
    fi
    return 0
}

# ============================================================================
# STEP 0: System Preparation
# ============================================================================

step_0_system_prep() {
    echo -e "${YELLOW}[0/10] System Preparation...${NC}"
    
    # Check OS
    if [[ "$OSTYPE" != "linux-gnu"* ]]; then
        echo -e "${RED}✗ Linux required${NC}"
        exit 1
    fi
    
    # Check Ubuntu
    if [ -f /etc/os-release ]; then
        . /etc/os-release
        if [[ "$ID" != "ubuntu" ]]; then
            echo -e "${RED}✗ Ubuntu required${NC}"
            exit 1
        fi
        echo -e "${GREEN}✓ Ubuntu $VERSION_ID detected${NC}"
    fi
    
    # Check disk space
    AVAILABLE_SPACE=$(df / | awk 'NR==2 {print $4}')
    REQUIRED_SPACE=$((${PROFILES_SPACE[$PROFILE]%GB} * 1024 * 1024))
    
    if [ "$AVAILABLE_SPACE" -lt "$REQUIRED_SPACE" ]; then
        echo -e "${RED}✗ Insufficient disk space${NC}"
        exit 1
    fi
    echo -e "${GREEN}✓ Disk space OK${NC}"
    
    # Update system
    echo "Updating system packages..."
    sudo apt-get update -qq
    sudo apt-get upgrade -y -qq
    sudo apt-get install -y -qq build-essential curl wget git
    
    echo -e "${GREEN}✓ System prepared${NC}"
}

# ============================================================================
# STEP 1: Install Docker
# ============================================================================

step_1_install_docker() {
    echo -e "\n${YELLOW}[1/10] Installing Docker...${NC}"
    
    # Check if already installed correctly
    if command -v docker &> /dev/null 2>/dev/null; then
        echo -e "${GREEN}✓ Docker already installed${NC}"
        docker --version
        return 0
    fi
    
    echo "Removing old Docker installations..."
    sudo apt-get remove -y docker-ce docker-ce-cli containerd.io docker-compose-plugin 2>/dev/null || true
    sudo apt-get purge -y docker-ce docker-ce-cli containerd.io docker-compose-plugin 2>/dev/null || true
    sudo apt-get autoremove -y -qq
    
    echo "Cleaning Docker directories..."
    sudo rm -rf /etc/docker
    sudo rm -rf /var/lib/docker
    sudo rm -rf /var/lib/containerd
    sudo rm -rf /etc/apt/sources.list.d/docker.list
    sudo rm -rf /etc/apt/keyrings/docker.gpg
    sudo rm -f /usr/bin/docker*
    sudo rm -f /usr/local/bin/docker*
    
    echo "Setting up Docker repository..."
    
    # Install prerequisites
    sudo apt-get install -y -qq ca-certificates curl gnupg lsb-release
    
    # Create keyrings directory
    sudo mkdir -p /etc/apt/keyrings
    
    # Add Docker GPG key
    curl -fsSL https://download.docker.com/linux/ubuntu/gpg | sudo gpg --dearmor -o /etc/apt/keyrings/docker.gpg 2>/dev/null
    
    # Add Docker repository
    echo "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/docker.gpg] https://download.docker.com/linux/ubuntu $(lsb_release -cs) stable" | sudo tee /etc/apt/sources.list.d/docker.list > /dev/null
    
    # Update package list
    sudo apt-get update -qq
    
    # Install Docker
    echo "Installing Docker packages..."
    sudo apt-get install -y -qq docker-ce docker-ce-cli containerd.io docker-buildx-plugin docker-compose-plugin
    
    # Verify installation
    sleep 2
    if ! command -v docker &> /dev/null 2>/dev/null; then
        echo -e "${RED}✗ Docker installation failed${NC}"
        echo "Checking for docker binary..."
        ls -la /usr/bin/docker* 2>/dev/null || echo "No docker binary found"
        exit 1
    fi
    
    echo -e "${GREEN}✓ Docker installed${NC}"
    docker --version
}

# ============================================================================
# STEP 2: Configure Docker Permissions
# ============================================================================

step_2_docker_permissions() {
    echo -e "\n${YELLOW}[2/10] Configuring Docker Permissions...${NC}"
    
    sudo groupadd docker 2>/dev/null || true
    sudo usermod -aG docker $USER
    sudo mkdir -p /var/run/docker 2>/dev/null || true
    sudo chown root:docker /var/run/docker 2>/dev/null || true
    sudo chmod 755 /var/run/docker 2>/dev/null || true
    
    echo -e "${YELLOW}⚠ IMPORTANT: You must logout and login to apply permissions!${NC}"
    echo -e "${YELLOW}   Commands:${NC}"
    echo -e "${YELLOW}   1. exit${NC}"
    echo -e "${YELLOW}   2. Login again${NC}"
    echo -e "${YELLOW}   3. Run: ./setup.sh standard${NC}"
    
    echo -e "${GREEN}✓ Docker permissions configured${NC}"
}

# ============================================================================
# STEP 3: Start Docker Service
# ============================================================================

step_3_start_docker() {
    echo -e "\n${YELLOW}[3/10] Starting Docker Service...${NC}"
    
    sudo systemctl start docker
    
    echo "Waiting for Docker..."
    for i in {1..30}; do
        if sudo docker ps &> /dev/null 2>&1; then
            echo -e "\n${GREEN}✓ Docker is ready${NC}"
            return 0
        fi
        echo -n "."
        sleep 1
    done
    
    echo -e "${RED}✗ Docker failed to start${NC}"
    exit 1
}

# ============================================================================
# STEP 4: Install NVIDIA Container Toolkit
# ============================================================================

step_4_nvidia_toolkit() {
    echo -e "\n${YELLOW}[4/10] Installing NVIDIA Container Toolkit...${NC}"
    
    # Check GPU
    if ! command -v nvidia-smi &> /dev/null; then
        echo -e "${RED}✗ NVIDIA GPU drivers not found${NC}"
        exit 1
    fi
    
    nvidia-smi | grep -i "NVIDIA GeForce\|Tesla" && echo -e "${GREEN}✓ NVIDIA GPU detected${NC}"
    
    # Install toolkit
    echo "Installing NVIDIA Container Toolkit..."
    sudo apt-get install -y -qq nvidia-container-toolkit 2>/dev/null || true
    
    # Configure Docker daemon
    sudo nvidia-ctk runtime configure --runtime=docker
    
    # Restart Docker daemon to apply changes
    echo "Restarting Docker daemon..."
    sudo systemctl restart docker
    sleep 5

    echo -e "${GREEN}✓ NVIDIA Container Toolkit installed${NC}"
    echo -e "${YELLOW}Note: GPU test will be performed after pulling images (step 7)${NC}"
}

# ============================================================================
# STEP 5: Install docker-compose
# ============================================================================

step_5_install_docker_compose() {
    echo -e "\n${YELLOW}[5/10] Installing docker-compose...${NC}"
    
    if command -v docker-compose &> /dev/null; then
        echo -e "${GREEN}✓ docker-compose already installed${NC}"
        docker-compose --version
        return 0
    fi
    
    sudo apt-get install -y -qq docker-compose-plugin
    
    echo -e "${GREEN}✓ docker-compose installed${NC}"
    docker compose version
}

# ============================================================================
# STEP 6: Prepare Project & Models
# ============================================================================

step_6_prepare_project() {
    echo -e "\n${YELLOW}[6/10] Preparing Project...${NC}"
    
    # Verify we're in the right directory
    if [ ! -f "docker-compose.yml" ]; then
        echo -e "${RED}✗ docker-compose.yml not found${NC}"
        echo "Please run from: ~/ai/rag-enterprise-complete/rag-enterprise-structure"
        exit 1
    fi
    
    echo -e "${GREEN}✓ docker-compose.yml found${NC}"
    
    # Create model directories
    echo "Creating model cache directories..."
    mkdir -p ~/.ollama
    mkdir -p ~/.cache/huggingface
    mkdir -p ~/.paddleocr
    
    chmod -R 755 ~/.ollama
    chmod -R 755 ~/.cache/huggingface
    chmod -R 755 ~/.paddleocr
    
    echo -e "${GREEN}✓ Model directories ready${NC}"
}

# ============================================================================
# STEP 6B: Configure docker-compose.yml based on profile
# ============================================================================

step_6b_configure_compose() {
    echo -e "\n${YELLOW}[6B/10] Configuring docker-compose.yml...${NC}"
    
    local EMBEDDING=${PROFILES_EMBEDDING[$PROFILE]}
    local LLM=${PROFILES_LLM[$PROFILE]}
    local THRESHOLD="0.5"
    
    # Adjust threshold based on embedding model
    case $EMBEDDING in
        "all-mpnet-base-v2")
            THRESHOLD="0.4"
            ;;
        "BAAI/bge-m3"|"bge-large-en-v1.5")
            THRESHOLD="0.35"
            ;;
        "instructor-large")
            THRESHOLD="0.3"
            ;;
    esac
    
    # Map LLM names to actual model names (già nel formato corretto per quantizzati)
    case $LLM in
        "mistral:7b-instruct-q4_K_M")
            LLM="mistral:7b-instruct-q4_K_M"
            ;;
        "qwen2.5:14b-instruct-q4_K_M")
            LLM="qwen2.5:14b-instruct-q4_K_M"
            ;;
        "qwen2.5:32b-instruct-q4_K_M")
            LLM="qwen2.5:32b-instruct-q4_K_M"
            ;;
        # Legacy support
        "qwen2.5:7b")
            LLM="qwen2.5:7b"
            ;;
        "qwen2.5:14b")
            LLM="qwen2.5:14b-instruct-q4_K_M"
            ;;
        "neural-chat")
            LLM="neural-chat:7b"
            ;;
        "mistral")
            LLM="mistral:7b-instruct-q4_K_M"
            ;;
        "llama2")
            LLM="llama3.1:8b"
            ;;
    esac
    
    # Update docker-compose.yml (use | as delimiter to avoid issues with / in model names)
    sed -i "s|LLM_MODEL: .*|LLM_MODEL: $LLM|" docker-compose.yml
    sed -i "s|EMBEDDING_MODEL: .*|EMBEDDING_MODEL: $EMBEDDING|" docker-compose.yml

    # Add RELEVANCE_THRESHOLD if not exists
    if ! grep -q "RELEVANCE_THRESHOLD:" docker-compose.yml; then
        sed -i "/EMBEDDING_MODEL:/a\      RELEVANCE_THRESHOLD: \"$THRESHOLD\"" docker-compose.yml
    else
        sed -i "s|RELEVANCE_THRESHOLD: .*|RELEVANCE_THRESHOLD: \"$THRESHOLD\"|" docker-compose.yml
    fi
    
    echo -e "${GREEN}✓ docker-compose.yml configured${NC}"
    echo -e "  LLM: $LLM"
    echo -e "  Embedding: $EMBEDDING"
    echo -e "  Threshold: $THRESHOLD"
}

# ============================================================================
# STEP 7: Pull Docker Images
# ============================================================================

step_7_pull_images() {
    echo -e "\n${YELLOW}[7/10] Pulling Docker Images...${NC}"

    echo "Pulling images (this may take a few minutes)..."

    echo -e "\n${CYAN}Pulling Ollama...${NC}"
    sudo docker pull ollama/ollama:latest

    echo -e "\n${CYAN}Pulling Qdrant...${NC}"
    sudo docker pull qdrant/qdrant:latest

    echo -e "\n${CYAN}Pulling NVIDIA CUDA (~1-2GB)...${NC}"
    sudo docker pull nvidia/cuda:12.9.0-runtime-ubuntu22.04

    echo -e "\n${GREEN}✓ All Docker images pulled${NC}"

    # Test GPU
    echo -e "\n${YELLOW}Testing GPU with Docker...${NC}"
    if sudo docker run --rm --gpus all nvidia/cuda:12.9.0-runtime-ubuntu22.04 nvidia-smi &> /dev/null; then
        echo -e "${GREEN}✓ NVIDIA GPU working correctly${NC}"
    else
        echo -e "${YELLOW}⚠ GPU test inconclusive (may work anyway)${NC}"
    fi
}

# ============================================================================
# STEP 8: Build Backend
# ============================================================================

step_8_build_backend() {
    echo -e "\n${YELLOW}[8/10] Building Backend Docker Image...${NC}"
    
    echo "Building (this may take 10-15 minutes)..."
    sudo docker compose build backend
    
    if sudo docker images | grep -q rag-enterprise-structure-backend; then
        echo -e "${GREEN}✓ Backend image built${NC}"
    else
        echo -e "${RED}✗ Backend build failed${NC}"
        exit 1
    fi
}

# ============================================================================
# STEP 9: Start Services
# ============================================================================

step_9_start_services() {
    echo -e "\n${YELLOW}[9/10] Starting Services...${NC}"
    
    sudo docker compose down -v 2>/dev/null || true
    
    echo "Starting containers..."
    sudo docker compose up -d
    
    sleep 10
    
    if sudo docker compose ps | grep -q "Up.*rag-backend"; then
        echo -e "${GREEN}✓ Services started${NC}"
    else
        echo -e "${RED}✗ Services failed to start${NC}"
        sudo docker compose logs backend | tail -30
        exit 1
    fi
}

# ============================================================================
# STEP 10: Pull Ollama Model & Final Verification
# ============================================================================

step_10_final_setup() {
    echo -e "\n${YELLOW}[10/10] Final Setup & Verification...${NC}"
    
    LLM_MODEL=${PROFILES_LLM[$PROFILE]}
    
	# Map model names to Ollama pull names (supporta quantizzati)
    case $LLM_MODEL in
        "mistral:7b-instruct-q4_K_M")
            OLLAMA_MODEL="mistral:7b-instruct-q4_K_M"
            ;;
        "qwen2.5:14b-instruct-q4_K_M")
            OLLAMA_MODEL="qwen2.5:14b-instruct-q4_K_M"
            ;;
        "qwen2.5:32b-instruct-q4_K_M")
            OLLAMA_MODEL="qwen2.5:32b-instruct-q4_K_M"
            ;;
        # Legacy support
        "qwen2.5:7b")
            OLLAMA_MODEL="qwen2.5:7b"
            ;;
        "qwen2.5:14b")
            OLLAMA_MODEL="qwen2.5:14b-instruct-q4_K_M"
            ;;
        "neural-chat")
            OLLAMA_MODEL="neural-chat:7b"
            ;;
        "mistral")
            OLLAMA_MODEL="mistral:7b-instruct-q4_K_M"
            ;;
        "llama2")
            OLLAMA_MODEL="llama3.1:8b"
            ;;
        *)
            OLLAMA_MODEL=$LLM_MODEL
            ;;
    esac
	
    # Wait for Ollama container to be ready
    echo "Waiting for Ollama container to be ready..."
    for i in {1..60}; do
        if sudo docker exec rag-ollama ollama list &> /dev/null 2>&1; then
            echo -e "${GREEN}✓ Ollama ready${NC}"
            break
        fi
        echo -n "."
        sleep 2
    done

    # Check if model is already pulled
    echo "Checking if $OLLAMA_MODEL is already available..."
    if sudo docker exec rag-ollama ollama list | grep -q "$OLLAMA_MODEL"; then
        echo -e "${GREEN}✓ Model $OLLAMA_MODEL already available, skipping download${NC}"
    else
        # Pull LLM model
        echo -e "\n${YELLOW}Pulling LLM model: $LLM_MODEL${NC}"
        echo "(This will take several minutes depending on model size)"
        
        if ! sudo docker exec rag-ollama ollama pull $OLLAMA_MODEL; then
            echo -e "${RED}✗ Failed to pull Ollama model${NC}"
            echo "Retrying..."
            sleep 5
            if ! sudo docker exec rag-ollama ollama pull $OLLAMA_MODEL; then
                echo -e "${RED}✗ Ollama model pull failed after retry${NC}"
                exit 1
            fi
        fi
        
        echo -e "${GREEN}✓ LLM model pulled${NC}"
    fi
  
    # Pull LLM model
    echo -e "\n${YELLOW}Pulling LLM model: $LLM_MODEL${NC}"
    echo "(This will take several minutes depending on model size)"
    
    if ! sudo docker exec rag-ollama ollama pull $OLLAMA_MODEL; then
        echo -e "${RED}✗ Failed to pull Ollama model${NC}"
        echo "Retrying..."
        sleep 5
        if ! sudo docker exec rag-ollama ollama pull $OLLAMA_MODEL; then
            echo -e "${RED}✗ Ollama model pull failed after retry${NC}"
            exit 1
        fi
    fi
    
    echo -e "${GREEN}✓ LLM model pulled${NC}"
    
    # Wait for backend health
    echo -e "\n${YELLOW}Waiting for backend to be healthy...${NC}"
    for i in {1..300}; do
        HEALTH=$(curl -s http://localhost:8000/health 2>/dev/null | grep -o '"status":"[^"]*"' || echo "")
        if [[ "$HEALTH" == *"healthy"* ]]; then
            echo -e "\n${GREEN}✓ Backend is healthy${NC}"
            break
        fi
        echo -n "."
        sleep 2
    done
    
    # Verify all services
    echo -e "\n${YELLOW}Verifying all services...${NC}"
    
    # Backend API
    if curl -s http://localhost:8000/health | grep -q "healthy"; then
        echo -e "${GREEN}✓ Backend API responding${NC}"
    else
        echo -e "${RED}✗ Backend API not responding${NC}"
        exit 1
    fi
    
    # Qdrant
    if curl -s http://localhost:6333/ | grep -q "version"; then
        echo -e "${GREEN}✓ Qdrant responding${NC}"
    else
        echo -e "${RED}✗ Qdrant not responding${NC}"
        exit 1
    fi
    
    # Ollama
    if sudo docker exec rag-ollama ollama list | grep -q "$OLLAMA_MODEL"; then
        echo -e "${GREEN}✓ Ollama model loaded${NC}"
    else
        echo -e "${RED}✗ Ollama model not loaded${NC}"
        exit 1
    fi
    
    # Frontend
    if curl -s http://localhost:3000 &> /dev/null 2>&1; then
        echo -e "${GREEN}✓ Frontend responding${NC}"
    else
        echo -e "${RED}✗ Frontend not responding${NC}"
        exit 1
    fi
    
    echo -e "\n${GREEN}✓ All services verified and ready!${NC}"
}

# ============================================================================
# PRINT SUCCESS
# ============================================================================

print_success() {
    echo -e "\n${GREEN}╔════════════════════════════════════════════════════════╗${NC}"
    echo -e "${GREEN}║   🎉 RAG ENTERPRISE ONLINE!                           ║${NC}"
    echo -e "${GREEN}║   Profile: $(echo $PROFILE | tr '[:lower:]' '[:upper:]')$(printf '%*s' $((40 - ${#PROFILE})))"${GREEN}"║${NC}"
    echo -e "${GREEN}╚════════════════════════════════════════════════════════╝${NC}"
    
    echo -e "\n${BLUE}Services available:${NC}"
    echo -e "  ${GREEN}Frontend:${NC}     http://localhost:3000"
    echo -e "  ${GREEN}Backend:${NC}      http://localhost:8000"
    echo -e "  ${GREEN}API Docs:${NC}     http://localhost:8000/docs"
    echo -e "  ${GREEN}Qdrant:${NC}       http://localhost:6333/dashboard"
    
    echo -e "\n${BLUE}Configuration:${NC}"
    echo -e "  ${YELLOW}Profile:${NC}        $(echo $PROFILE | tr '[:lower:]' '[:upper:]')"
    echo -e "  ${YELLOW}Embedding:${NC}      ${PROFILES_EMBEDDING[$PROFILE]}"
    echo -e "  ${YELLOW}LLM:${NC}            ${PROFILES_LLM[$PROFILE]}"
    
    echo -e "\n${BLUE}Useful commands:${NC}"
    echo -e "  ${YELLOW}View logs:${NC}    docker compose logs -f"
    echo -e "  ${YELLOW}Stop:${NC}         docker compose down"
    echo -e "  ${YELLOW}Restart:${NC}      docker compose restart"
    echo -e "  ${YELLOW}Status:${NC}       docker compose ps"
    
    echo -e "\n${GREEN}✓ Setup complete!${NC}\n"
}

# ============================================================================
# MAIN EXECUTION
# ============================================================================

main() {
    export PATH="/usr/local/bin:/usr/bin:/usr/sbin:/bin:$PATH"
    hash -r
    
    print_header
    print_profile_info
    
    [ $START_STEP -le 0 ] && step_0_system_prep
    [ $START_STEP -le 1 ] && step_1_install_docker
    [ $START_STEP -le 2 ] && step_2_docker_permissions
    [ $START_STEP -le 3 ] && step_3_start_docker
    [ $START_STEP -le 4 ] && step_4_nvidia_toolkit
    [ $START_STEP -le 5 ] && step_5_install_docker_compose
    [ $START_STEP -le 6 ] && step_6_prepare_project
    [ $START_STEP -le 6 ] && step_6b_configure_compose
    [ $START_STEP -le 7 ] && step_7_pull_images
    [ $START_STEP -le 8 ] && step_8_build_backend
    [ $START_STEP -le 9 ] && step_9_start_services
    [ $START_STEP -le 10 ] && step_10_final_setup
    
    print_success
}

main