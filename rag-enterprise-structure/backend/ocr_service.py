"""
OCR Service - Tika + Tesseract Fallback
"""
import logging
import subprocess
import requests
import time
import xml.etree.ElementTree as ET
from pathlib import Path
import re
import os

logger = logging.getLogger(__name__)

class OCRService:
    TIKA_URL = "http://localhost:9998"
    MIME_TYPES = {
        '.pdf': 'application/pdf',
        '.txt': 'text/plain',
        '.docx': 'application/vnd.openxmlformats-officedocument.wordprocessingml.document',
        '.doc': 'application/msword',
        '.pptx': 'application/vnd.openxmlformats-officedocument.presentationml.presentation',
        '.ppt': 'application/vnd.ms-powerpoint',
        '.xlsx': 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet',
        '.xls': 'application/vnd.ms-excel',
        '.odt': 'application/vnd.oasis.opendocument.text',
        '.rtf': 'application/rtf',
        '.html': 'text/html',
        '.htm': 'text/html',
        '.xml': 'application/xml',
        '.json': 'application/json',
        '.csv': 'text/csv',
        '.md': 'text/markdown',
    }
    
    def __init__(self):
        logger.info("Initializing OCR Service...")
        self.tika_ready = False
        self.tika_process = None
        
        self._aggressive_kill_tika()
        self._start_tika()
        logger.info("✅ OCR Service ready")
    
    def _aggressive_kill_tika(self):
        try:
            result1 = subprocess.run(['pkill', '-9', '-f', 'java.*tika'], 
                         capture_output=True, timeout=5)
            print(f"Kill java.*tika result: {result1.returncode}")
        
            result2 = subprocess.run(['pkill', '-9', '-f', '9998'], 
                         capture_output=True, timeout=5)
            print(f"Kill 9998 result: {result2.returncode}")
        
            time.sleep(2)
        except Exception as e:
            print(f"Kill error: {e}")
    
    def _start_tika(self):
        logger.info("Starting Tika...")
        self.tika_process = subprocess.Popen(
            ['java', '-jar', '/opt/tika-server.jar', '-h', '0.0.0.0', '-p', '9998'],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            preexec_fn=os.setsid
        )
        logger.info(f"Tika PID: {self.tika_process.pid}")
        logger.info(f"Testing URL: {self.TIKA_URL}/version")
        
        logger.info("Waiting for Tika startup (60 sec)...")
        for i in range(60):
            try:
                response = requests.get(f"{self.TIKA_URL}/version", timeout=1)
                if response.status_code == 200:
                    logger.info(f"✅ Tika ready at {i}s")
                    self.tika_ready = True
                    return
            except Exception as e:
                if i % 10 == 0:
                    logger.warning(f"Attempt {i}/60 - {type(e).__name__}: {str(e)}")
            time.sleep(1)
        
        logger.error(f"❌ Tika startup timeout! URL: {self.TIKA_URL}")
        raise RuntimeError("Tika not available")
    
    def _get_mime_type(self, file_path: str) -> str:
        ext = Path(file_path).suffix.lower()
        return self.MIME_TYPES.get(ext, 'application/octet-stream')
    
    def extract_text(self, file_path: str) -> str:
        try:
            logger.info(f"Extracting: {Path(file_path).name}")
            ext = Path(file_path).suffix.lower()
            
            if ext in ['.txt', '.md', '.csv']:
                try:
                    with open(file_path, 'r', encoding='utf-8') as f:
                        text = f.read()
                    if text and len(text.strip()) > 0:
                        logger.info(f"✅ {len(text)} chars (direct)")
                        return text.strip()
                except Exception as e:
                    logger.warning(f"Direct read failed: {str(e)}")
            
            logger.info(f"Tika ready: {self.tika_ready}")
            
            if self.tika_ready:
                try:
                    logger.info(f"Opening file: {file_path}")
                    with open(file_path, 'rb') as f:
                        file_data = f.read()
                    logger.info(f"File size: {len(file_data)} bytes")
                    
                    mime_type = self._get_mime_type(file_path)
                    logger.info(f"MIME type: {mime_type}")
                    logger.info(f"Sending to Tika: {self.TIKA_URL}/tika")
                    
                    response = requests.put(
                        f"{self.TIKA_URL}/tika",
                        data=file_data,
                        headers={
                            'Content-Type': mime_type,
                            'Accept-Charset': 'utf-8'  # Forza UTF-8 nella risposta
                        },
                        timeout=600
                    )

                    # Forza encoding UTF-8 sulla risposta
                    response.encoding = 'utf-8'

                    logger.info(f"Tika response status: {response.status_code}")
                    logger.info(f"Tika response encoding: {response.encoding}")
                    logger.info(f"Tika response length: {len(response.text)} chars")
                    
                    if response.status_code == 200:
                        text = self._extract_text_from_tika_xml(response.text)
                        if text and len(text.strip()) > 100:
                            logger.info(f"✅ {len(text)} chars (Tika)")
                            return text
                except Exception as e:
                    logger.error(f"Tika request error: {type(e).__name__}: {str(e)}")
            
            # 🔧 FALLBACK A TESSERACT se Tika non ha estratto abbastanza
            logger.warning("⚠️  Tika extraction insufficient, trying Tesseract...")
            tesseract_text = self._extract_with_tesseract(file_path)
            if tesseract_text and len(tesseract_text.strip()) > 0:
                logger.info(f"✅ {len(tesseract_text)} chars (Tesseract fallback)")
                return tesseract_text
            
            logger.warning("⚠️  No extraction worked")
            return ""
        except Exception as e:
            logger.error(f"Extract error: {str(e)}")
            return ""
    
    def _extract_text_from_tika_xml(self, xml_text: str) -> str:
        try:
            logger.info(f"XML length: {len(xml_text)} chars")
            
            if xml_text.startswith('\ufeff'):
                xml_text = xml_text[1:]
            
            xml_text = re.sub(r'&#0;', '', xml_text)
            xml_text = re.sub(r'&#[0-9]+;', '', xml_text)
            
            root = ET.fromstring(xml_text)
            logger.info(f"XML parsed successfully")
            
            ns = {'xhtml': 'http://www.w3.org/1999/xhtml'}
            
            body = root.find('.//xhtml:body', ns)
            if body is not None:
                text = ''.join(body.itertext()).strip()
                logger.info(f"Found xhtml:body with {len(text)} chars")
                if text:
                    return text
            
            body = root.find('.//body')
            if body is not None:
                text = ''.join(body.itertext()).strip()
                logger.info(f"Found body with {len(text)} chars")
                if text:
                    return text
            
            text = ''.join(root.itertext()).strip()
            logger.info(f"Got all text: {len(text)} chars")
            return text if text else ""
            
        except Exception as e:
            logger.error(f"XML error: {type(e).__name__}: {str(e)}")
            return ""
    
    def _extract_with_tesseract(self, file_path: str) -> str:
        """Fallback: estrae testo usando Tesseract direttamente"""
        try:
            import pytesseract
            from pdf2image import convert_from_path
            
            logger.info(f"🔍 Tesseract: convertendo PDF a immagini...")
            images = convert_from_path(file_path)
            logger.info(f"📄 {len(images)} pagine convertite")
            
            text = ""
            for i, img in enumerate(images):
                logger.info(f"  OCR pagina {i+1}/{len(images)}...")
                page_text = pytesseract.image_to_string(img, lang='ita+eng')
                text += page_text + "\n"
            
            logger.info(f"✅ Tesseract estratto {len(text)} chars")
            logger.info(f"📋 TESTO TESSERACT:\n{text[:1000]}")
            return text
            
        except ImportError as e:
            logger.error(f"❌ Modulo mancante: {e}")
            return ""
        except Exception as e:
            logger.error(f"❌ Tesseract failed: {type(e).__name__}: {str(e)}")
            return ""