# STEP 1: Stop everything
sudo docker compose down -v
sudo systemctl stop docker.socket docker.service

# STEP 2: Rimuovi Docker completamente
sudo apt-get remove -y docker-ce docker-ce-cli containerd.io docker-buildx-plugin docker-compose-plugin
sudo apt-get purge -y docker-ce docker-ce-cli containerd.io docker-buildx-plugin docker-compose-plugin
sudo apt-get autoremove -y

# STEP 3: Rimuovi configurazioni Docker
sudo rm -rf /etc/docker
sudo rm -rf /var/lib/docker
sudo rm -rf /var/lib/containerd
sudo rm -rf ~/.docker
sudo rm -rf /etc/apt/sources.list.d/docker.list
sudo rm -rf /etc/apt/keyrings/docker.gpg

# STEP 4: Rimuovi docker group
sudo groupdel docker 2>/dev/null || true

# STEP 5: Verifica
echo "✓ Docker completamente rimosso"
docker --version 2>&1 || echo "✓ Confirmed: Docker not found"

echo -e "\n✅ Sistema pulito!\n"
