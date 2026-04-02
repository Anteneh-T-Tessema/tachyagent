import torch
from sentence_transformers import SentenceTransformer
import faiss
import numpy as np
from typing import List, Tuple, Optional
import json
import os

class RAGSystem:
    def __init__(self, embedding_model_name: str = "all-MiniLM-L6-v2"):
        """
        Initialize the RAG system with embedding model and FAISS index
        """
        # Initialize the sentence transformer model for embeddings
        self.embedding_model = SentenceTransformer(embedding_model_name)
        
        # Initialize FAISS index
        self.index = None
        self.documents = []
        self.document_ids = []
        
    def add_documents(self, documents: List[str], document_ids: Optional[List[str]] = None):
        """
        Add documents to the knowledge base
        
        Args:
            documents: List of document texts
            document_ids: Optional list of document IDs (if not provided, auto-generated)
        """
        if document_ids is None:
            document_ids = [f"doc_{i}" for i in range(len(documents))]
            
        # Create embeddings for documents
        embeddings = self.embedding_model.encode(documents)
        
        # Initialize FAISS index
        if self.index is None:
            dimension = embeddings.shape[1]
            self.index = faiss.IndexFlatIP(dimension)  # Inner product for cosine similarity
        
        # Add embeddings to index
        self.index.add(np.array(embeddings, dtype=np.float32))
        
        # Store documents and their IDs
        self.documents.extend(documents)
        self.document_ids.extend(document_ids)
        
        print(f"Added {len(documents)} documents to knowledge base")
        
    def retrieve(self, query: str, k: int = 3) -> List[Tuple[str, float, str]]:
        """
        Retrieve most relevant documents for a query
        
        Returns:
            List of tuples (document, similarity_score, document_id)
        """
        if self.index is None:
            raise ValueError("No documents added to the knowledge base")
            
        # Create query embedding
        query_embedding = self.embedding_model.encode([query])
        
        # Search in FAISS index
        distances, indices = self.index.search(np.array(query_embedding, dtype=np.float32), k)
        
        # Return documents with their similarity scores and IDs
        results = []
        for i, (distance, idx) in enumerate(zip(distances[0], indices[0])):
            if idx < len(self.documents):
                results.append((self.documents[idx], distance, self.document_ids[idx]))
        
        return results
    
    def generate_response(self, query: str, retrieved_docs: List[str]) -> str:
        """
        Generate a response using retrieved documents
        This is a simplified version - in practice, you'd use a proper LLM
        """
        # Simple concatenation approach
        context = " ".join(retrieved_docs)
        
        # In a real implementation, you would use a language model here
        # For this example, we'll return a simple response
        return f"Based on the retrieved information: {context[:200]}... Answering your query: '{query}'"
    
    def query(self, query: str, k: int = 3) -> str:
        """
        Complete RAG query process: retrieve + generate
        """
        # Retrieve relevant documents
        retrieved = self.retrieve(query, k)
        
        # Extract just the documents for response generation
        retrieved_docs = [doc for doc, _, _ in retrieved]
        
        # Generate response
        response = self.generate_response(query, retrieved_docs)
        
        return response
    
    def save_index(self, index_path: str, docs_path: str):
        """
        Save the FAISS index and documents to disk
        """
        if self.index is not None:
            faiss.write_index(self.index, index_path)
            
        # Save documents
        with open(docs_path, 'w') as f:
            json.dump({
                'documents': self.documents,
                'document_ids': self.document_ids
            }, f)
    
    def load_index(self, index_path: str, docs_path: str):
        """
        Load the FAISS index and documents from disk
        """
        # Load FAISS index
        self.index = faiss.read_index(index_path)
        
        # Load documents
        with open(docs_path, 'r') as f:
            data = json.load(f)
            self.documents = data['documents']
            self.document_ids = data['document_ids']

# Example usage
if __name__ == "__main__":
    # Initialize RAG system
    rag = RAGSystem()
    
    # Sample documents
    documents = [
        "The Eiffel Tower is located in Paris, France. It was built in 1889.",
        "Albert Einstein was a German theoretical physicist who developed the theory of relativity.",
        "The capital of Japan is Tokyo. It is located on the island of Honshu.",
        "Python is a high-level programming language. It was created by Guido van Rossum.",
        "The Earth has a circumference of approximately 40,075 kilometers at the equator."
    ]
    
    # Add documents to knowledge base
    rag.add_documents(documents)
    
    # Query
    query = "Where is the Eiffel Tower?"
    
    # Complete RAG process
    response = rag.query(query, k=2)
    print(f"Query: {query}")
    print(f"Response: {response}")
    
    # Show retrieved documents
    retrieved = rag.retrieve(query, k=2)
    print("\nRetrieved documents:")
    for doc, score, doc_id in retrieved:
        print(f"Score: {score:.4f} - {doc}")
    
    # Save the index and documents
    rag.save_index("rag_index.faiss", "rag_documents.json")
    print("\nIndex and documents saved successfully!")
    
    # Load the index and documents
    new_rag = RAGSystem()
    new_rag.load_index("rag_index.faiss", "rag_documents.json")
    print("Index and documents loaded successfully!")
    
    # Test query on loaded system
    response = new_rag.query("Who developed the theory of relativity?", k=1)
    print(f"\nLoaded system query: {response}")