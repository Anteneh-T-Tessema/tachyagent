import torch
from sentence_transformers import SentenceTransformer
import faiss
import numpy as np
from typing import List, Tuple

class SimpleRAG:
    def __init__(self):
        # Initialize the sentence transformer model for embeddings
        self.model = SentenceTransformer('all-MiniLM-L6-v2')
        
        # Initialize FAISS index
        self.index = None
        self.documents = []
        
    def add_documents(self, documents: List[str]):
        """Add documents to the knowledge base"""
        # Create embeddings for documents
        embeddings = self.model.encode(documents)
        
        # Initialize FAISS index
        dimension = embeddings.shape[1]
        self.index = faiss.IndexFlatIP(dimension)  # Inner product for cosine similarity
        
        # Add embeddings to index
        self.index.add(np.array(embeddings, dtype=np.float32))
        
        # Store documents
        self.documents = documents
        
    def retrieve(self, query: str, k: int = 3) -> List[Tuple[str, float]]:
        """Retrieve most relevant documents for a query"""
        # Create query embedding
        query_embedding = self.model.encode([query])
        
        # Search in FAISS index
        distances, indices = self.index.search(np.array(query_embedding, dtype=np.float32), k)
        
        # Return documents with their similarity scores
        results = []
        for i, (distance, idx) in enumerate(zip(distances[0], indices[0])):
            if idx < len(self.documents):
                results.append((self.documents[idx], distance))
        
        return results
    
    def generate_response(self, query: str, retrieved_docs: List[str]) -> str:
        """Generate a response using retrieved documents"""
        # Simple concatenation approach
        context = " ".join(retrieved_docs)
        prompt = f"Based on the following information: {context}\n\nAnswer this question: {query}"
        
        # In a real implementation, you would use a language model here
        # For this example, we'll return a simple response
        return f"Query: {query}\nContext: {context[:200]}..."

# Example usage
if __name__ == "__main__":
    # Initialize RAG system
    rag = SimpleRAG()
    
    # Sample documents
    documents = [
        "The Eiffel Tower is located in Paris, France.",
        "Albert Einstein was a German theoretical physicist.",
        "The capital of Japan is Tokyo.",
        "Python is a high-level programming language.",
        "The Earth has a circumference of approximately 40,075 kilometers."
    ]
    
    # Add documents to knowledge base
    rag.add_documents(documents)
    
    # Query
    query = "Where is the Eiffel Tower?"
    
    # Retrieve relevant documents
    retrieved = rag.retrieve(query, k=2)
    
    print("Retrieved documents:")
    for doc, score in retrieved:
        print(f"Score: {score:.4f} - {doc}")
    
    # Generate response
    response = rag.generate_response(query, [doc for doc, _ in retrieved])
    print(f"\nResponse: {response}")