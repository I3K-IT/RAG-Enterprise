import React, { useState, useEffect } from 'react'
import axios from 'axios'
import './App.css'

function App() {
  const [status, setStatus] = useState('checking')
  const [backendHealth, setBackendHealth] = useState(null)
  const [uploadProgress, setUploadProgress] = useState(0)
  const [uploading, setUploading] = useState(false)
  const [query, setQuery] = useState('')
  const [querying, setQuerying] = useState(false)
  const [results, setResults] = useState(null)
  const [documents, setDocuments] = useState([])

  // Check backend health on mount
  useEffect(() => {
    checkBackendHealth()
    const interval = setInterval(checkBackendHealth, 10000)
    return () => clearInterval(interval)
  }, [])

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

  const handleFileUpload = async (e) => {
    const file = e.target.files[0]
    if (!file) return

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
      
      setDocuments([...documents, {
        name: file.name,
        size: file.size,
        chunks: response.data.chunks_count,
        timestamp: new Date().toLocaleString()
      }])

      setUploadProgress(0)
      alert(`✅ File caricato: ${response.data.filename}\nElaborazione in corso...`)
    } catch (error) {
      console.error('Upload error:', error)
      alert(`❌ Errore upload: ${error.response?.data?.detail || error.message}`)
    } finally {
      setUploading(false)
      e.target.value = ''
    }
  }

  const handleDirectoryUpload = async (e) => {
    const files = e.target.files
    if (!files.length) return

    setUploading(true)
    let totalChunks = 0

    for (let file of files) {
      setUploadProgress(Math.round((Array.from(files).indexOf(file) / files.length) * 100))

      const formData = new FormData()
      formData.append('file', file)

      try {
        const response = await axios.post('http://localhost:8000/api/documents/upload', formData)
        totalChunks += response.data.chunks_count
      } catch (error) {
        console.error(`Errore uploading ${file.name}:`, error)
      }
    }

    setUploadProgress(0)
    alert(`✅ Directory caricata: ${totalChunks} chunks totali`)
    setUploading(false)
    e.target.value = ''
    
    // Refresh documents
    checkBackendHealth()
  }

  const handleQuery = async (e) => {
    e.preventDefault()
    if (!query.trim()) return

    setQuerying(true)
    setResults(null)

    try {
      const response = await axios.post('http://localhost:8000/api/query', {
        query: query,
        top_k: 5
      })

      setResults(response.data)
    } catch (error) {
      console.error('Query error:', error)
      alert(`❌ Errore query: ${error.response?.data?.detail || error.message}`)
    } finally {
      setQuerying(false)
    }
  }

  return (
    <div className="min-h-screen bg-gradient-to-br from-slate-900 to-slate-800">
      <div className="container mx-auto px-4 py-8">
        
        {/* Header */}
        <div className="mb-8">
          <h1 className="text-4xl font-bold text-white mb-2">🚀 RAG Enterprise</h1>
          <p className="text-slate-300">Retrieval Augmented Generation Platform</p>
        </div>

        {/* Status Bar */}
        <div className={`mb-8 p-4 rounded-lg ${
          status === 'ready' ? 'bg-green-900/30 border border-green-500' :
          status === 'error' ? 'bg-red-900/30 border border-red-500' :
          'bg-yellow-900/30 border border-yellow-500'
        }`}>
          <div className="flex items-center justify-between">
            <div>
              <p className="text-white font-semibold">
                {status === 'ready' ? '✅ Sistema Pronto' :
                 status === 'error' ? '❌ Backend Non Disponibile' :
                 '⏳ Checking...'}
              </p>
              {backendHealth && (
                <p className="text-sm text-slate-300 mt-1">
                  LLM: {backendHealth.configuration.llm_model} | 
                  Embedding: {backendHealth.configuration.embedding_model} | 
                  Threshold: {backendHealth.configuration.relevance_threshold}
                </p>
              )}
            </div>
            <button 
              onClick={checkBackendHealth}
              className="px-4 py-2 bg-blue-600 hover:bg-blue-700 text-white rounded"
            >
              Refresh
            </button>
          </div>
        </div>

        <div className="grid grid-cols-1 lg:grid-cols-3 gap-8">
          
          {/* Upload Section */}
          <div className="lg:col-span-1 space-y-6">
            
            {/* Single File Upload */}
            <div className="bg-slate-800 border border-slate-700 rounded-lg p-6">
              <h2 className="text-xl font-bold text-white mb-4">📁 Upload File</h2>
              <label className="block">
                <div className="border-2 border-dashed border-slate-600 rounded-lg p-8 text-center cursor-pointer hover:border-blue-500 transition">
                  <input
                    type="file"
                    onChange={handleFileUpload}
                    disabled={uploading}
                    className="hidden"
                    accept=".pdf,.docx,.txt,.doc,.pptx,.xlsx"
                  />
                  <p className="text-slate-300">
                    {uploading ? `Uploading... ${uploadProgress}%` : 'Click to upload file'}
                  </p>
                  <p className="text-xs text-slate-400 mt-2">PDF, DOCX, TXT, PPT, XLS</p>
                </div>
              </label>
              {uploadProgress > 0 && (
                <div className="mt-4 bg-slate-700 rounded-full h-2">
                  <div 
                    className="bg-blue-500 h-2 rounded-full transition-all"
                    style={{ width: `${uploadProgress}%` }}
                  ></div>
                </div>
              )}
            </div>

            {/* Directory Upload */}
            <div className="bg-slate-800 border border-slate-700 rounded-lg p-6">
              <h2 className="text-xl font-bold text-white mb-4">📂 Upload Directory</h2>
              <label className="block">
                <div className="border-2 border-dashed border-slate-600 rounded-lg p-8 text-center cursor-pointer hover:border-green-500 transition">
                  <input
                    type="file"
                    onChange={handleDirectoryUpload}
                    disabled={uploading}
                    className="hidden"
                    webkitdirectory="true"
                    accept=".pdf,.docx,.txt,.doc,.pptx,.xlsx"
                  />
                  <p className="text-slate-300">
                    {uploading ? `Uploading...` : 'Click to upload directory'}
                  </p>
                  <p className="text-xs text-slate-400 mt-2">Upload intera cartella</p>
                </div>
              </label>
            </div>

            {/* Documents List */}
            {documents.length > 0 && (
              <div className="bg-slate-800 border border-slate-700 rounded-lg p-6">
                <h3 className="text-lg font-bold text-white mb-4">📋 Documenti</h3>
                <div className="space-y-2 max-h-64 overflow-y-auto">
                  {documents.map((doc, idx) => (
                    <div key={idx} className="bg-slate-700 p-3 rounded text-sm">
                      <p className="text-white font-semibold truncate">{doc.name}</p>
                      <p className="text-slate-400 text-xs">
                        {doc.chunks} chunks • {new Date(doc.timestamp).toLocaleTimeString()}
                      </p>
                    </div>
                  ))}
                </div>
              </div>
            )}
          </div>

          {/* Query Section */}
          <div className="lg:col-span-2">
            <div className="bg-slate-800 border border-slate-700 rounded-lg p-6">
              <h2 className="text-xl font-bold text-white mb-6">🔍 Query RAG</h2>
              
              <form onSubmit={handleQuery} className="mb-6">
                <div className="flex gap-2">
                  <input
                    type="text"
                    value={query}
                    onChange={(e) => setQuery(e.target.value)}
                    placeholder="Cosa vuoi cercare nei documenti?"
                    disabled={querying}
                    className="flex-1 bg-slate-700 text-white placeholder-slate-400 rounded-lg px-4 py-3 focus:outline-none focus:ring-2 focus:ring-blue-500"
                  />
                  <button
                    type="submit"
                    disabled={querying}
                    className="px-6 py-3 bg-blue-600 hover:bg-blue-700 disabled:bg-slate-600 text-white rounded-lg font-semibold transition"
                  >
                    {querying ? '⏳ Searching...' : '🚀 Search'}
                  </button>
                </div>
              </form>

              {/* Results */}
              {results && (
                <div className="space-y-6">
                  
                  {/* Answer */}
                  <div className="bg-slate-700 rounded-lg p-6 border border-slate-600">
                    <h3 className="text-white font-bold mb-3">💡 Risposta</h3>
                    <p className="text-slate-100 leading-relaxed">
                      {results.answer}
                    </p>
                  </div>

                  {/* Sources */}
                  {results.sources && results.sources.length > 0 && (
                    <div className="bg-slate-700 rounded-lg p-6 border border-slate-600">
                      <h3 className="text-white font-bold mb-4">📚 Fonti ({results.sources.length})</h3>
                      <div className="space-y-3">
                        {results.sources.map((source, idx) => (
                          <div key={idx} className="bg-slate-600 rounded p-4">
                            <div className="flex justify-between items-start mb-2">
                              <p className="text-white font-semibold truncate">
                                {source.document_id}
                              </p>
                              <span className="bg-green-600 text-white px-3 py-1 rounded text-sm font-bold">
                                {(source.similarity * 100).toFixed(1)}%
                              </span>
                            </div>
                            <p className="text-slate-300 text-sm leading-relaxed">
                              {source.text}
                            </p>
                          </div>
                        ))}
                      </div>
                    </div>
                  )}
                </div>
              )}
            </div>
          </div>

        </div>
      </div>
    </div>
  )
}

export default App
