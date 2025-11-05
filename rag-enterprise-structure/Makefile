.PHONY: help install install-all install-db install-backend install-frontend logs logs-backend logs-frontend stop restart clean health

help:
	@echo "RAG Enterprise - Makefile Commands"
	@echo ""
	@echo "INSTALLAZIONE:"
	@echo "  make install-all         Installa tutto (DB + Backend + Frontend)"
	@echo "  make install-db          Installa solo Database"
	@echo "  make install-backend     Installa Backend (con DB locale)"
	@echo "  make install-frontend    Installa Frontend"
	@echo ""
	@echo "GESTIONE:"
	@echo "  make logs                Visualizza log tutti servizi"
	@echo "  make logs-backend        Visualizza log backend"
	@echo "  make logs-frontend       Visualizza log frontend"
	@echo "  make stop                Arresta servizi"
	@echo "  make restart             Riavvia servizi"
	@echo "  make clean               Rimuovi container e volumi"
	@echo "  make health              Verifica salute servizi"
	@echo ""
	@echo "URL SERVIZI:"
	@echo "  Frontend:  http://localhost:3000"
	@echo "  Backend:   http://localhost:8000"
	@echo "  Docs:      http://localhost:8000/docs"

install-all:
	@echo "🚀 Installazione completa..."
	@chmod +x setup.sh
	@./setup.sh

install-db:
	@echo "🚀 Installazione Database..."
	@chmod +x setup-database-only.sh
	@./setup-database-only.sh

install-backend:
	@echo "🚀 Installazione Backend..."
	@chmod +x setup-backend-only.sh
	@./setup-backend-only.sh

install-frontend:
	@echo "🚀 Installazione Frontend..."
	@chmod +x setup-frontend-only.sh
	@./setup-frontend-only.sh

logs:
	@docker-compose logs -f

logs-backend:
	@docker-compose logs -f backend

logs-frontend:
	@docker-compose logs -f frontend

stop:
	@echo "Arresto servizi..."
	@docker-compose down

restart:
	@echo "Riavvio servizi..."
	@docker-compose restart

clean:
	@echo "Pulizia completa (rimozione container e volumi)..."
	@docker-compose down -v
	@echo "✓ Pulizia completata"

health:
	@echo "Verifica salute servizi..."
	@curl -s http://localhost:8000/health | python3 -m json.tool || echo "❌ Backend non raggiungibile"
	@echo ""
	@docker-compose ps
