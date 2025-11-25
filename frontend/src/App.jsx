import React, { useState, useEffect, useRef } from 'react'
import axios from 'axios'
import './App.css'

// Configurazione logo e branding (personalizzabile per ogni cliente)
const BRANDING = {
  clientLogo: null, // URL del logo cliente, null = usa nome testuale
  clientName: 'RAG Enterprise',
  primaryColor: '#3b82f6', // blue-500
  poweredBy: 'I3K Technologies',
  version: 'v1.0'
}

function App() {
  // Backend status
  const [status, setStatus] = useState('checking')
  const [backendHealth, setBackendHealth] = useState(null)

  // Conversazioni
  const [conversations, setConversations] = useState([])
  const [currentConversationId, setCurrentConversationId] = useState(null)
  const [messages, setMessages] = useState([])

  // Input query
  const [query, setQuery] = useState('')
  const [querying, setQuerying] = useState(false)

  // Documenti
  const [documents, setDocuments] = useState([])
  const [loadingDocuments, setLoadingDocuments] = useState(false)
  const [uploadProgress, setUploadProgress] = useState(0)
  const [uploading, setUploading] = useState(false)

  // UI state
  const [showConversationsSidebar, setShowConversationsSidebar] = useState(true)
  const [showDocumentsSidebar, setShowDocumentsSidebar] = useState(true)

  const messagesEndRef = useRef(null)
  const fileInputRef = useRef(null)

  // Scroll to bottom when new messages arrive
  const scrollToBottom = () => {
    messagesEndRef.current?.scrollIntoView({ behavior: 'smooth' })
  }

  useEffect(scrollToBottom, [messages])

  // Initialize: load conversations and check backend
  useEffect(() => {
    loadConversationsFromStorage()
    checkBackendHealth()
    fetchDocuments()

    const interval = setInterval(checkBackendHealth, 30000)
    return () => clearInterval(interval)
  }, [])

  // Load conversations from localStorage
  const loadConversationsFromStorage = () => {
    try {
      const stored = localStorage.getItem('rag_conversations')
      if (stored) {
        const parsed = JSON.parse(stored)
        setConversations(parsed)

        // Ripristina l'ultima conversazione attiva
        const lastActiveId = localStorage.getItem('rag_current_conversation')
        if (lastActiveId && parsed.find(c => c.id === lastActiveId)) {
          loadConversation(lastActiveId)
        } else if (parsed.length > 0) {
          loadConversation(parsed[0].id)
        }
      } else {
        // Crea conversazione iniziale
        createNewConversation()
      }
    } catch (error) {
      console.error('Errore caricamento conversazioni:', error)
      createNewConversation()
    }
  }

  // Save conversations to localStorage
  const saveConversationsToStorage = (convs) => {
    localStorage.setItem('rag_conversations', JSON.stringify(convs))
  }

  // Create new conversation
  const createNewConversation = () => {
    const newConv = {
      id: Date.now().toString(),
      title: 'Nuova Conversazione',
      messages: [],
      createdAt: new Date().toISOString()
    }

    const updated = [newConv, ...conversations]
    setConversations(updated)
    saveConversationsToStorage(updated)
    loadConversation(newConv.id)
  }

  // Load conversation
  const loadConversation = (convId) => {
    const conv = conversations.find(c => c.id === convId)
    if (conv) {
      setCurrentConversationId(convId)
      setMessages(conv.messages || [])
      localStorage.setItem('rag_current_conversation', convId)
    }
  }

  // Delete conversation
  const deleteConversation = (convId) => {
    if (conversations.length === 1) {
      alert('Non puoi eliminare l\'ultima conversazione')
      return
    }

    const updated = conversations.filter(c => c.id !== convId)
    setConversations(updated)
    saveConversationsToStorage(updated)

    if (currentConversationId === convId) {
      loadConversation(updated[0].id)
    }
  }

  // Update conversation title (first message preview)
  const updateConversationTitle = (convId, firstMessage) => {
    const updated = conversations.map(c => {
      if (c.id === convId && c.title === 'Nuova Conversazione') {
        return {
          ...c,
          title: firstMessage.substring(0, 50) + (firstMessage.length > 50 ? '...' : '')
        }
      }
      return c
    })
    setConversations(updated)
    saveConversationsToStorage(updated)
  }

  // Update conversation messages
  const updateConversationMessages = (convId, newMessages) => {
    const updated = conversations.map(c =>
      c.id === convId ? { ...c, messages: newMessages } : c
    )
    setConversations(updated)
    saveConversationsToStorage(updated)
  }

  // Check backend health
  const checkBackendHealth = async () => {
    try {
      const response = await axios.get('http://localhost:8000/health')
      setBackendHealth(response.data)
      setStatus('ready')
    } catch (error) {
      setBackendHealth(null)
      setStatus('error')
    }
  }

  // Fetch documents from backend
  const fetchDocuments = async () => {
    setLoadingDocuments(true)
    try {
      const response = await axios.get('http://localhost:8000/api/documents')
      setDocuments(response.data.documents || [])
    } catch (error) {
      console.error('Errore fetch documenti:', error)
    } finally {
      setLoadingDocuments(false)
    }
  }

  // Check if document already exists
  const checkDuplicateDocument = (filename) => {
    return documents.some(doc => doc.filename === filename)
  }

  // Handle file upload
  const handleFileUpload = async (e) => {
    const file = e.target.files[0]
    if (!file) return

    // Controllo duplicati
    if (checkDuplicateDocument(file.name)) {
      const confirm = window.confirm(
        `⚠️ Il documento "${file.name}" è già presente.\n\nVuoi caricarlo comunque?`
      )
      if (!confirm) {
        e.target.value = ''
        return
      }
    }

    setUploading(true)
    setUploadProgress(0)

    const formData = new FormData()
    formData.append('file', file)

    try {
      const config = {
        onUploadProgress: (progressEvent) => {
          const percent = Math.round(
            (progressEvent.loaded * 100) / progressEvent.total
          )
          setUploadProgress(percent)
        }
      }

      const response = await axios.post('http://localhost:8000/api/documents/upload', formData, config)

      alert(`✅ File caricato: ${response.data.filename}\n${response.data.chunks_count} chunks generati`)

      // Refresh documents list
      fetchDocuments()

    } catch (error) {
      console.error('Upload error:', error)
      alert(`❌ Errore upload: ${error.response?.data?.detail || error.message}`)
    } finally {
      setUploading(false)
      setUploadProgress(0)
      e.target.value = ''
    }
  }

  // Handle query submit
  const handleQuery = async (e) => {
    e.preventDefault()
    if (!query.trim() || querying) return

    const userMessage = {
      role: 'user',
      content: query,
      timestamp: new Date().toISOString()
    }

    // Add user message immediately
    const updatedMessages = [...messages, userMessage]
    setMessages(updatedMessages)
    updateConversationMessages(currentConversationId, updatedMessages)

    // Update conversation title if first message
    if (updatedMessages.length === 1) {
      updateConversationTitle(currentConversationId, query)
    }

    setQuery('')
    setQuerying(true)

    try {
      const response = await axios.post('http://localhost:8000/api/query', {
        query: userMessage.content,
        top_k: 5,
        user_id: currentConversationId
      })

      const assistantMessage = {
        role: 'assistant',
        content: response.data.answer,
        sources: response.data.sources || [],
        timestamp: new Date().toISOString()
      }

      const finalMessages = [...updatedMessages, assistantMessage]
      setMessages(finalMessages)
      updateConversationMessages(currentConversationId, finalMessages)

    } catch (error) {
      console.error('Query error:', error)

      const errorMessage = {
        role: 'assistant',
        content: `❌ Errore: ${error.response?.data?.detail || error.message}`,
        error: true,
        timestamp: new Date().toISOString()
      }

      const finalMessages = [...updatedMessages, errorMessage]
      setMessages(finalMessages)
      updateConversationMessages(currentConversationId, finalMessages)
    } finally {
      setQuerying(false)
    }
  }

  // Delete document
  const handleDeleteDocument = async (documentId) => {
    if (!window.confirm('Sei sicuro di voler eliminare questo documento?')) {
      return
    }

    try {
      await axios.delete(`http://localhost:8000/api/documents/${documentId}`)
      alert('✅ Documento eliminato')
      fetchDocuments()
    } catch (error) {
      console.error('Delete error:', error)
      alert(`❌ Errore eliminazione: ${error.response?.data?.detail || error.message}`)
    }
  }

  return (
    <div className="flex flex-col h-screen bg-gradient-to-br from-slate-900 to-slate-800">

      {/* HEADER */}
      <header className="bg-slate-800 border-b border-slate-700 px-6 py-4 flex items-center justify-between">
        <div className="flex items-center gap-4">
          {BRANDING.clientLogo ? (
            <img src={BRANDING.clientLogo} alt="Logo" className="h-10" />
          ) : (
            <h1 className="text-2xl font-bold text-white">{BRANDING.clientName}</h1>
          )}
        </div>

        {/* Status indicator */}
        <div className="flex items-center gap-4">
          <div className="flex items-center gap-2">
            <div className={`w-3 h-3 rounded-full ${
              status === 'ready' ? 'bg-green-500' :
              status === 'error' ? 'bg-red-500' :
              'bg-yellow-500'
            }`} />
            <span className="text-sm text-slate-300">
              {status === 'ready' ? 'Online' :
               status === 'error' ? 'Offline' :
               'Checking...'}
            </span>
          </div>

          {backendHealth && (
            <button
              onClick={checkBackendHealth}
              className="text-sm text-slate-400 hover:text-white transition"
              title={`LLM: ${backendHealth.configuration?.llm_model || 'N/A'}`}
            >
              🔄 Refresh
            </button>
          )}
        </div>
      </header>

      {/* MAIN CONTENT */}
      <div className="flex flex-1 overflow-hidden">

        {/* SIDEBAR CONVERSAZIONI */}
        {showConversationsSidebar && (
          <aside className="w-64 bg-slate-800 border-r border-slate-700 flex flex-col">
            <div className="p-4 border-b border-slate-700">
              <button
                onClick={createNewConversation}
                className="w-full py-2 px-4 bg-blue-600 hover:bg-blue-700 text-white rounded-lg font-semibold transition"
              >
                + Nuova Chat
              </button>
            </div>

            <div className="flex-1 overflow-y-auto p-2 space-y-1">
              {conversations.map(conv => (
                <div
                  key={conv.id}
                  className={`group flex items-center justify-between p-3 rounded cursor-pointer transition ${
                    currentConversationId === conv.id
                      ? 'bg-slate-700 text-white'
                      : 'text-slate-300 hover:bg-slate-700/50'
                  }`}
                  onClick={() => loadConversation(conv.id)}
                >
                  <span className="truncate flex-1 text-sm">{conv.title}</span>
                  {conversations.length > 1 && (
                    <button
                      onClick={(e) => {
                        e.stopPropagation()
                        deleteConversation(conv.id)
                      }}
                      className="opacity-0 group-hover:opacity-100 text-red-400 hover:text-red-300 text-xs ml-2"
                    >
                      🗑️
                    </button>
                  )}
                </div>
              ))}
            </div>

            <div className="p-2 border-t border-slate-700">
              <button
                onClick={() => setShowConversationsSidebar(false)}
                className="w-full py-1 text-xs text-slate-400 hover:text-white"
              >
                ◀ Chiudi
              </button>
            </div>
          </aside>
        )}

        {/* CHAT AREA */}
        <main className="flex-1 flex flex-col">

          {/* Toggle sidebar button (if hidden) */}
          {!showConversationsSidebar && (
            <button
              onClick={() => setShowConversationsSidebar(true)}
              className="absolute top-20 left-4 p-2 bg-slate-700 text-white rounded-lg shadow-lg hover:bg-slate-600 z-10"
            >
              ▶
            </button>
          )}

          {/* Messages area */}
          <div className="flex-1 overflow-y-auto p-6 space-y-4">
            {messages.length === 0 ? (
              <div className="flex items-center justify-center h-full">
                <div className="text-center text-slate-400">
                  <h2 className="text-2xl font-bold mb-2">👋 Ciao!</h2>
                  <p>Inizia una conversazione oppure carica dei documenti per iniziare.</p>
                </div>
              </div>
            ) : (
              messages.map((msg, idx) => (
                <div
                  key={idx}
                  className={`flex ${msg.role === 'user' ? 'justify-end' : 'justify-start'}`}
                >
                  <div
                    className={`max-w-3xl rounded-lg p-4 ${
                      msg.role === 'user'
                        ? 'bg-blue-600 text-white'
                        : msg.error
                        ? 'bg-red-900/30 border border-red-500 text-red-200'
                        : 'bg-slate-700 text-slate-100'
                    }`}
                  >
                    <p className="whitespace-pre-wrap leading-relaxed">{msg.content}</p>

                    {/* Sources */}
                    {msg.sources && msg.sources.length > 0 && (
                      <div className="mt-4 pt-4 border-t border-slate-600 space-y-2">
                        <p className="text-sm font-semibold text-slate-300">
                          📚 Fonti ({msg.sources.length}):
                        </p>
                        {msg.sources.map((source, sidx) => (
                          <div key={sidx} className="bg-slate-600 rounded p-2 text-sm">
                            <div className="flex justify-between items-center gap-2">
                              <a
                                href={`http://localhost:8000/api/documents/${source.document_id}/download`}
                                download
                                className="text-blue-300 hover:text-blue-200 underline truncate flex-1"
                                title={source.filename || source.document_id}
                              >
                                {source.filename || source.document_id}
                              </a>
                              <span className="bg-green-600 text-white px-2 py-1 rounded text-xs font-bold">
                                {source.similarity_score ? (source.similarity_score * 100).toFixed(1) : 'N/A'}%
                              </span>
                            </div>
                          </div>
                        ))}
                      </div>
                    )}

                    <p className="text-xs text-slate-400 mt-2">
                      {new Date(msg.timestamp).toLocaleTimeString('it-IT')}
                    </p>
                  </div>
                </div>
              ))
            )}

            {querying && (
              <div className="flex justify-start">
                <div className="bg-slate-700 rounded-lg p-4 text-slate-300">
                  <div className="flex items-center gap-2">
                    <div className="animate-spin">⏳</div>
                    <span>Ricerca in corso...</span>
                  </div>
                </div>
              </div>
            )}

            <div ref={messagesEndRef} />
          </div>

          {/* Input area */}
          <div className="border-t border-slate-700 p-4 bg-slate-800">
            <form onSubmit={handleQuery} className="flex gap-2">
              <input
                type="text"
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder="Scrivi la tua domanda..."
                disabled={querying || status !== 'ready'}
                className="flex-1 bg-slate-700 text-white placeholder-slate-400 rounded-lg px-4 py-3 focus:outline-none focus:ring-2 focus:ring-blue-500 disabled:opacity-50"
              />
              <button
                type="submit"
                disabled={querying || !query.trim() || status !== 'ready'}
                className="px-6 py-3 bg-blue-600 hover:bg-blue-700 disabled:bg-slate-600 disabled:cursor-not-allowed text-white rounded-lg font-semibold transition"
              >
                {querying ? '⏳' : '🚀'}
              </button>
            </form>
          </div>
        </main>

        {/* SIDEBAR DOCUMENTI */}
        {showDocumentsSidebar && (
          <aside className="w-80 bg-slate-800 border-l border-slate-700 flex flex-col">
            <div className="p-4 border-b border-slate-700">
              <h2 className="text-lg font-bold text-white mb-3">📁 Documenti</h2>

              <input
                ref={fileInputRef}
                type="file"
                onChange={handleFileUpload}
                disabled={uploading}
                className="hidden"
                accept=".pdf,.docx,.txt,.doc,.pptx,.xlsx"
              />

              <button
                onClick={() => fileInputRef.current?.click()}
                disabled={uploading}
                className="w-full py-2 px-4 bg-green-600 hover:bg-green-700 disabled:bg-slate-600 text-white rounded-lg font-semibold transition"
              >
                {uploading ? `⏳ ${uploadProgress}%` : '+ Carica File'}
              </button>

              {uploadProgress > 0 && uploading && (
                <div className="mt-2 bg-slate-700 rounded-full h-2">
                  <div
                    className="bg-green-500 h-2 rounded-full transition-all"
                    style={{ width: `${uploadProgress}%` }}
                  />
                </div>
              )}
            </div>

            <div className="flex-1 overflow-y-auto p-3">
              {loadingDocuments ? (
                <p className="text-center text-slate-400 py-4">Caricamento...</p>
              ) : documents.length === 0 ? (
                <p className="text-center text-slate-400 py-4 text-sm">
                  Nessun documento caricato
                </p>
              ) : (
                <div className="space-y-2">
                  {documents.map((doc, idx) => (
                    <div
                      key={idx}
                      className="bg-slate-700 rounded-lg p-3 group hover:bg-slate-600 transition"
                    >
                      <div className="flex items-start justify-between gap-2">
                        <div className="flex-1 min-w-0">
                          <p className="text-white text-sm font-semibold truncate" title={doc.filename}>
                            {doc.filename}
                          </p>
                          <p className="text-slate-400 text-xs mt-1">
                            {doc.chunks_count || 0} chunks
                          </p>
                        </div>

                        <button
                          onClick={() => handleDeleteDocument(doc.document_id)}
                          className="opacity-0 group-hover:opacity-100 text-red-400 hover:text-red-300 text-xs transition"
                          title="Elimina documento"
                        >
                          🗑️
                        </button>
                      </div>
                    </div>
                  ))}
                </div>
              )}
            </div>

            <div className="p-2 border-t border-slate-700">
              <button
                onClick={fetchDocuments}
                className="w-full py-2 text-sm text-slate-400 hover:text-white transition"
              >
                🔄 Aggiorna Lista
              </button>
              <button
                onClick={() => setShowDocumentsSidebar(false)}
                className="w-full py-1 text-xs text-slate-400 hover:text-white mt-1"
              >
                Chiudi ▶
              </button>
            </div>
          </aside>
        )}

        {/* Toggle documents sidebar button (if hidden) */}
        {!showDocumentsSidebar && (
          <button
            onClick={() => setShowDocumentsSidebar(true)}
            className="absolute top-20 right-4 p-2 bg-slate-700 text-white rounded-lg shadow-lg hover:bg-slate-600 z-10"
          >
            ◀
          </button>
        )}
      </div>

      {/* FOOTER */}
      <footer className="bg-slate-900 border-t border-slate-700 px-6 py-3">
        <div className="flex items-center justify-between text-xs text-slate-400">
          <div>
            <span>Powered by </span>
            <span className="font-semibold text-blue-400">{BRANDING.poweredBy}</span>
            <span className="mx-2">•</span>
            <span>{BRANDING.version}</span>
          </div>
          <div>
            <a
              href="#"
              className="hover:text-white transition"
              onClick={(e) => {
                e.preventDefault()
                alert('⚠️ Disclaimer:\n\nQuesto sistema utilizza intelligenza artificiale per analizzare documenti e fornire risposte. Le informazioni fornite devono essere verificate e non sostituiscono la consulenza professionale.\n\n© I3K Technologies - Tutti i diritti riservati.')
              }}
            >
              Disclaimer
            </a>
            <span className="mx-2">•</span>
            <a href="#" className="hover:text-white transition">Privacy</a>
          </div>
        </div>
      </footer>
    </div>
  )
}

export default App
